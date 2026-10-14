//! Bounded startup terminal probes and one continuous raw-input decoder.
//!
//! Once raw mode is enabled, a probe owns stdin until its incremental decoder
//! becomes the live reader. Validated terminal replies are removed before they
//! can become input; every other byte follows that same decoder into the app.

use std::io;
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::layout::Position;
use termwiz::input::{
    InputEvent as TermwizEvent, InputParser, KeyCode as TermwizKeyCode, Modifiers as TermwizMods,
};

use crate::appearance::BackgroundColor;
use crate::brand::{MultiplexerKind, TerminalContext, TerminalName};
use crate::da2::{self, Da2Version};
use crate::xtversion::{self, XtVersion};

/// OSC 11 asks a terminal for its effective background color.
const OSC11_QUERY: &[u8] = b"\x1b]11;?\x07";
/// Device status report request for the current 1-based cursor position.
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
const MAX_OSC11_REPLY_LEN: usize = 64;
const MAX_DA2_REPLY_LEN: usize = 32;
const MAX_INPUT_ESCAPE_LEN: usize = 256;
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Startup probe timing and memory limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StartupProbeConfig {
    /// Initial response window after all selected queries are flushed.
    pub timeout: Duration,
    /// Extra bounded drain window for a response that races the initial timeout.
    pub late_reply_grace: Duration,
    /// Maximum raw input retained while probing.
    pub max_bytes: usize,
}

impl Default for StartupProbeConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(500),
            late_reply_grace: Duration::from_millis(100),
            max_bytes: 768,
        }
    }
}

/// Sanitized terminal facts discovered during a startup session.
///
/// `context` is the immutable environment-derived identification. The other
/// fields are populated only by replies to queries explicitly armed for that
/// direct terminal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalInfo {
    pub context: TerminalContext,
    pub background: Option<BackgroundColor>,
    pub da2: Option<Da2Version>,
    pub xtversion: Option<XtVersion>,
}

impl TerminalInfo {
    pub fn from_context(context: TerminalContext) -> Self {
        Self {
            context,
            background: None,
            da2: None,
            xtversion: None,
        }
    }

    /// Resolves explicit appearance override, probed background, then OS
    /// appearance using the terminal crate's public precedence policy.
    pub fn appearance(&self) -> Option<crate::appearance::SystemAppearance> {
        crate::appearance::detect_with_background(self.background)
    }
}

/// A bounded read preserves bytes even when an I/O error follows them.
#[derive(Debug)]
pub struct ProbeRead {
    pub bytes: Vec<u8>,
    pub error: Option<io::Error>,
}

impl ProbeRead {
    pub fn complete(bytes: Vec<u8>) -> Self {
        Self { bytes, error: None }
    }

    pub fn partial(bytes: Vec<u8>, error: io::Error) -> Self {
        Self {
            bytes,
            error: Some(error),
        }
    }
}

/// Injectable I/O seam for controlled-PTY and protocol tests.
///
/// Implementations must return bytes read before an error. Classification
/// happens in [`StartupProbe`], not in the I/O adapter.
pub trait StartupProbeIo {
    fn write_query(&mut self, query: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
    fn read_bounded(&mut self, config: StartupProbeConfig) -> ProbeRead;
}

/// Data learned by the bounded, injectable probe.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupProbeOutcome {
    pub info: TerminalInfo,
    /// 0-based physical cursor position captured for an inline viewport.
    pub cursor_position: Option<Position>,
    preserved_input: Vec<u8>,
}

impl StartupProbeOutcome {
    /// Decodes all non-reply bytes for delivery through the normal input path.
    ///
    /// This is for a finite injected probe. The live terminal path instead
    /// retains one decoder across startup and runtime input.
    pub fn take_events(&mut self) -> Vec<Event> {
        let bytes = std::mem::take(&mut self.preserved_input);
        ProbeEventDecoder::decode_complete(&bytes)
    }

    pub fn appearance(&self) -> Option<crate::appearance::SystemAppearance> {
        self.info.appearance()
    }

    /// Returns retained non-reply bytes to a caller with its own decoder.
    pub fn take_preserved_input(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.preserved_input)
    }
}

/// Runs direct-terminal startup probes selected for one terminal context.
#[derive(Clone, Copy, Debug)]
pub struct StartupProbe {
    context: TerminalContext,
    config: StartupProbeConfig,
    capture_cursor: bool,
}

impl StartupProbe {
    pub const fn new(context: TerminalContext, config: StartupProbeConfig) -> Self {
        Self {
            context,
            config,
            capture_cursor: false,
        }
    }

    pub fn for_context(context: TerminalContext) -> Self {
        Self::new(context, StartupProbeConfig::default())
    }

    /// Adds a DSR cursor-position query for an inline viewport whose origin
    /// must be known without a second stdin reader.
    pub const fn with_cursor_position(mut self) -> Self {
        self.capture_cursor = true;
        self
    }

    /// Sends only safe queries, reads one bounded batch, and removes only
    /// validated replies. A write or read failure is a probe skip after its
    /// already-observed input has been preserved.
    pub fn run<I: StartupProbeIo>(&self, io: &mut I) -> StartupProbeOutcome {
        let queries = QuerySet::for_context(self.context, self.capture_cursor);
        let mut preserved_input = Vec::new();
        let mut replies = ReplyFilter::new(self.context, queries);
        if queries.is_empty() {
            return StartupProbeOutcome {
                info: replies.info,
                cursor_position: replies.cursor_position,
                preserved_input,
            };
        }

        for query in queries.iter() {
            if io.write_query(query).is_err() {
                return StartupProbeOutcome {
                    info: replies.info,
                    cursor_position: replies.cursor_position,
                    preserved_input,
                };
            }
        }
        if io.flush().is_err() {
            return StartupProbeOutcome {
                info: replies.info,
                cursor_position: replies.cursor_position,
                preserved_input,
            };
        }

        let read = io.read_bounded(self.config);
        replies.push(&read.bytes, |bytes| {
            preserved_input.extend_from_slice(bytes)
        });
        // The injectable seam has no next read. Do not lose a partial user
        // sequence just because it happened to resemble an armed response.
        replies.finish(|bytes| preserved_input.extend_from_slice(bytes));
        StartupProbeOutcome {
            info: replies.info,
            cursor_position: replies.cursor_position,
            preserved_input,
        }
    }

    /// Opens the persistent Unix reader after raw mode has been enabled.
    ///
    /// A failed capability probe is a no-op. A requested cursor position is
    /// different: inline viewport construction cannot safely invent an origin,
    /// so its writer, reader, and deadline errors are surfaced to the caller.
    #[cfg(unix)]
    pub fn run_live(&self) -> io::Result<Option<StartupProbeSession>> {
        use std::io::{IsTerminal, Write};
        use std::time::Instant;

        use rustix::event::{PollFd, PollFlags, Timespec, poll};

        let queries = QuerySet::for_context(self.context, self.capture_cursor);
        if queries.is_empty() {
            return Ok(None);
        }
        let input = std::io::stdin();
        let mut output = std::io::stdout();
        if !input.is_terminal() || !output.is_terminal() {
            return Ok(None);
        }

        let mut reader = match UnixInput::open(self.context, queries) {
            Ok(reader) => reader,
            Err(error) if queries.cursor => return Err(error),
            Err(_) => return Ok(None),
        };
        for query in queries.iter() {
            if let Err(error) = output.write_all(query) {
                return if queries.cursor { Err(error) } else { Ok(None) };
            }
        }
        if let Err(error) = output.flush() {
            return if queries.cursor { Err(error) } else { Ok(None) };
        }

        let deadline = Instant::now()
            .checked_add(self.config.timeout + self.config.late_reply_grace)
            .unwrap_or_else(Instant::now);
        let mut initial_events = Vec::new();
        let mut consumed = 0;
        while consumed < self.config.max_bytes && !reader.replies.all_resolved() {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            let timeout = Timespec {
                tv_sec: remaining.as_secs().try_into().unwrap_or(i64::MAX),
                tv_nsec: remaining.subsec_nanos().into(),
            };
            let mut fds = [PollFd::new(&reader.input, PollFlags::IN)];
            match poll(&mut fds, Some(&timeout)) {
                Ok(0) => break,
                Ok(_) if !fds[0].revents().intersects(PollFlags::IN | PollFlags::HUP) => break,
                Ok(_) => match reader.read_limited(self.config.max_bytes - consumed) {
                    Ok((read, events)) => {
                        consumed += read;
                        initial_events.extend(events);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(error) if queries.cursor => return Err(error),
                    Err(_) => break,
                },
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) if queries.cursor => return Err(error.into()),
                Err(_) => break,
            }
        }

        if queries.cursor && reader.replies.cursor_position.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out waiting for terminal cursor position",
            ));
        }
        let info = reader.replies.info.clone();
        let cursor_position = reader.replies.cursor_position;
        reader.replies.take_update();
        Ok(Some(StartupProbeSession {
            info,
            cursor_position,
            input: reader,
            initial_events,
        }))
    }
}

#[derive(Clone, Copy)]
struct QuerySet {
    osc11: bool,
    da2: bool,
    xtversion: bool,
    cursor: bool,
}

impl QuerySet {
    fn for_context(context: TerminalContext, cursor: bool) -> Self {
        Self {
            osc11: osc11_is_safe(&context),
            da2: da2_is_safe(&context),
            xtversion: xtversion::probe_is_safe(&context),
            cursor,
        }
    }

    const fn is_empty(self) -> bool {
        !self.osc11 && !self.da2 && !self.xtversion && !self.cursor
    }

    fn iter(self) -> impl Iterator<Item = &'static [u8]> {
        [
            self.osc11.then_some(OSC11_QUERY),
            self.da2.then_some(da2::QUERY),
            self.xtversion.then_some(xtversion::QUERY),
            self.cursor.then_some(CURSOR_POSITION_QUERY),
        ]
        .into_iter()
        .flatten()
    }
}

fn osc11_is_safe(context: &TerminalContext) -> bool {
    context.multiplexer == MultiplexerKind::Undetected
        && matches!(
            context.brand,
            TerminalName::Kitty
                | TerminalName::Ghostty
                | TerminalName::Iterm2
                | TerminalName::WezTerm
                | TerminalName::Alacritty
                | TerminalName::Rio
                | TerminalName::Foot
                | TerminalName::Vte
        )
}

fn da2_is_safe(context: &TerminalContext) -> bool {
    context.multiplexer == MultiplexerKind::Undetected && context.brand == TerminalName::Alacritty
}

enum Classification {
    Valid(usize, Reply),
    Pending,
    NotReply,
}

enum Reply {
    Background(BackgroundColor),
    Da2(Da2Version),
    XtVersion(XtVersion),
    Cursor(Position),
}

/// Removes only replies to queries that were armed for this process terminal.
///
/// The small pending buffer is retained across the startup/live boundary, so a
/// reply split across those reads is never replayed as ordinary keystrokes.
struct ReplyFilter {
    info: TerminalInfo,
    cursor_position: Option<Position>,
    queries: QuerySet,
    pending: Vec<u8>,
    pasting: bool,
    changed: Option<TerminalInfo>,
}

impl ReplyFilter {
    fn new(context: TerminalContext, queries: QuerySet) -> Self {
        Self {
            info: TerminalInfo::from_context(context),
            cursor_position: None,
            queries,
            pending: Vec::new(),
            pasting: false,
            changed: None,
        }
    }

    fn all_resolved(&self) -> bool {
        (!self.queries.osc11 || self.info.background.is_some())
            && (!self.queries.da2 || self.info.da2.is_some())
            && (!self.queries.xtversion || self.info.xtversion.is_some())
            && (!self.queries.cursor || self.cursor_position.is_some())
    }

    fn push<F: FnMut(&[u8])>(&mut self, bytes: &[u8], mut emit_input: F) {
        if self.pending.is_empty() && !bytes.contains(&b'\x1b') {
            emit_input(bytes);
            return;
        }

        self.pending.extend_from_slice(bytes);
        let mut raw = std::mem::take(&mut self.pending);
        let mut cursor = 0;
        while cursor < raw.len() {
            let input = &raw[cursor..];
            if self.pasting {
                if input[0] != b'\x1b' {
                    let end = input
                        .iter()
                        .position(|byte| *byte == b'\x1b')
                        .map_or(raw.len(), |offset| cursor + offset);
                    emit_input(&raw[cursor..end]);
                    cursor = end;
                    continue;
                }
                if input.starts_with(PASTE_END) {
                    emit_input(PASTE_END);
                    cursor += PASTE_END.len();
                    self.pasting = false;
                    continue;
                }
                if PASTE_END.starts_with(input) {
                    raw.drain(..cursor);
                    self.pending = raw;
                    return;
                }
                let end = input[1..]
                    .iter()
                    .position(|byte| *byte == b'\x1b')
                    .map_or(raw.len(), |offset| cursor + offset + 1);
                emit_input(&raw[cursor..end]);
                cursor = end;
                continue;
            }

            if input[0] != b'\x1b' {
                let end = input
                    .iter()
                    .position(|byte| *byte == b'\x1b')
                    .map_or(raw.len(), |offset| cursor + offset);
                emit_input(&raw[cursor..end]);
                cursor = end;
                continue;
            }
            if input.starts_with(PASTE_START) {
                emit_input(PASTE_START);
                cursor += PASTE_START.len();
                self.pasting = true;
                continue;
            }
            if PASTE_START.starts_with(input) {
                raw.drain(..cursor);
                self.pending = raw;
                return;
            }

            match self.classify(input) {
                Classification::Valid(len, reply) => {
                    self.record(reply);
                    cursor += len;
                }
                Classification::Pending => {
                    raw.drain(..cursor);
                    self.pending = raw;
                    return;
                }
                Classification::NotReply => {
                    // Feed the escape separately so the incremental decoder
                    // can join it with the bytes that follow in this or a
                    // subsequent read.
                    emit_input(&raw[cursor..cursor + 1]);
                    cursor += 1;
                }
            }
        }
        raw.clear();
        self.pending = raw;
    }

    fn finish<F: FnMut(&[u8])>(&mut self, mut emit_input: F) {
        if !self.pending.is_empty() {
            emit_input(&self.pending);
            self.pending.clear();
        }
    }

    fn has_pending_control(&self) -> bool {
        !self.pasting && self.pending == b"\x1b"
    }

    fn take_update(&mut self) -> Option<TerminalInfo> {
        self.changed.take()
    }

    fn record(&mut self, reply: Reply) {
        let changed = match reply {
            Reply::Background(background) if self.info.background != Some(background) => {
                self.info.background = Some(background);
                true
            }
            Reply::Da2(da2) if self.info.da2 != Some(da2) => {
                self.info.da2 = Some(da2);
                true
            }
            Reply::XtVersion(xtversion) if self.info.xtversion.as_ref() != Some(&xtversion) => {
                self.info.xtversion = Some(xtversion);
                true
            }
            Reply::Cursor(position) => {
                self.cursor_position = Some(position);
                self.queries.cursor = false;
                false
            }
            _ => false,
        };
        if changed {
            self.changed = Some(self.info.clone());
        }
    }

    fn classify(&self, bytes: &[u8]) -> Classification {
        if self.queries.osc11 {
            match classify_osc11(bytes) {
                Classification::NotReply => {}
                result => return result,
            }
        }
        if self.queries.da2 {
            match classify_da2(bytes) {
                Classification::NotReply => {}
                result => return result,
            }
        }
        if self.queries.xtversion {
            match classify_xtversion(bytes) {
                Classification::NotReply => {}
                result => return result,
            }
        }
        if self.queries.cursor {
            match classify_cursor_position(bytes) {
                Classification::NotReply => {}
                result => return result,
            }
        }
        Classification::NotReply
    }
}

fn starts_or_is_prefix(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.starts_with(prefix) || prefix.starts_with(bytes)
}

fn classify_osc11(bytes: &[u8]) -> Classification {
    const PREFIX: &[u8] = b"\x1b]11;";
    if !starts_or_is_prefix(bytes, PREFIX) {
        return Classification::NotReply;
    }
    if !bytes.starts_with(PREFIX) {
        return Classification::Pending;
    }
    match osc11_reply_len(bytes) {
        Some(len) if len <= MAX_OSC11_REPLY_LEN => {
            BackgroundColor::parse_osc11_reply(&bytes[..len])
                .map_or(Classification::NotReply, |background| {
                    Classification::Valid(len, Reply::Background(background))
                })
        }
        Some(_) => Classification::NotReply,
        None if bytes.len() > MAX_OSC11_REPLY_LEN => Classification::NotReply,
        None => Classification::Pending,
    }
}

fn classify_da2(bytes: &[u8]) -> Classification {
    const PREFIX: &[u8] = b"\x1b[>0;";
    if !starts_or_is_prefix(bytes, PREFIX) {
        return Classification::NotReply;
    }
    if !bytes.starts_with(PREFIX) {
        return Classification::Pending;
    }
    match da2_reply_len(bytes) {
        Some(len) if len <= MAX_DA2_REPLY_LEN => Da2Version::parse(&bytes[..len])
            .map_or(Classification::NotReply, |version| {
                Classification::Valid(len, Reply::Da2(version))
            }),
        Some(_) => Classification::NotReply,
        None if bytes.len() > MAX_DA2_REPLY_LEN => Classification::NotReply,
        None => Classification::Pending,
    }
}

fn classify_xtversion(bytes: &[u8]) -> Classification {
    const PREFIX: &[u8] = b"\x1bP>|";
    if !starts_or_is_prefix(bytes, PREFIX) {
        return Classification::NotReply;
    }
    if !bytes.starts_with(PREFIX) {
        return Classification::Pending;
    }
    match xtversion_reply_len(bytes) {
        Some(len) => XtVersion::parse(&bytes[..len]).map_or(Classification::NotReply, |version| {
            Classification::Valid(len, Reply::XtVersion(version))
        }),
        None if bytes.len() > 4 + xtversion::MAX_PAYLOAD_LEN + 2 => Classification::NotReply,
        None => Classification::Pending,
    }
}

fn classify_cursor_position(bytes: &[u8]) -> Classification {
    const PREFIX: &[u8] = b"\x1b[";
    if !starts_or_is_prefix(bytes, PREFIX) {
        return Classification::NotReply;
    }
    if bytes.len() <= PREFIX.len() {
        return Classification::Pending;
    }
    if !bytes[2].is_ascii_digit() {
        return Classification::NotReply;
    }
    let Some(end) = bytes[2..]
        .iter()
        .position(|byte| !byte.is_ascii_digit() && *byte != b';')
        .map(|offset| offset + 2)
    else {
        return if bytes.len() <= MAX_DA2_REPLY_LEN {
            Classification::Pending
        } else {
            Classification::NotReply
        };
    };
    if end >= MAX_DA2_REPLY_LEN || bytes[end] != b'R' {
        return Classification::NotReply;
    }
    let body = &bytes[2..end];
    let Some((row, column)) = std::str::from_utf8(body)
        .ok()
        .and_then(|body| body.split_once(';'))
        .and_then(|(row, column)| Some((row.parse::<u16>().ok()?, column.parse::<u16>().ok()?)))
        .filter(|(row, column)| *row > 0 && *column > 0)
    else {
        return Classification::NotReply;
    };
    Classification::Valid(
        end + 1,
        Reply::Cursor(Position::new(
            column.saturating_sub(1),
            row.saturating_sub(1),
        )),
    )
}

fn osc11_reply_len(bytes: &[u8]) -> Option<usize> {
    let body = bytes.strip_prefix(b"\x1b]11;")?;
    for (index, byte) in body.iter().enumerate() {
        if *byte == b'\x07' {
            return Some(6 + index);
        }
        if *byte == b'\x1b' && body.get(index + 1) == Some(&b'\\') {
            return Some(7 + index);
        }
    }
    None
}

fn da2_reply_len(bytes: &[u8]) -> Option<usize> {
    let end = bytes.iter().position(|byte| *byte == b'c')?;
    (end < MAX_DA2_REPLY_LEN).then_some(end + 1)
}

fn xtversion_reply_len(bytes: &[u8]) -> Option<usize> {
    let body = bytes.strip_prefix(b"\x1bP>|")?;
    body.windows(2)
        .position(|window| window == b"\x1b\\")
        .filter(|index| *index <= xtversion::MAX_PAYLOAD_LEN)
        .map(|index| 4 + index + 2)
}

/// The live startup result owns the only reader that may continue consuming
/// stdin. It must be handed to the runtime instead of creating `EventStream`.
#[cfg(unix)]
pub struct StartupProbeSession {
    pub info: TerminalInfo,
    /// Present only when `with_cursor_position` was requested.
    pub cursor_position: Option<Position>,
    input: UnixInput,
    initial_events: Vec<Event>,
}

#[cfg(unix)]
impl StartupProbeSession {
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.initial_events)
    }

    pub fn into_input(self) -> UnixInput {
        self.input
    }
}

/// Independent nonblocking TTY reader carrying the probe reply filter and one
/// incremental input decoder for the entire terminal session.
#[cfg(unix)]
pub struct UnixInput {
    input: rustix::fd::OwnedFd,
    replies: ReplyFilter,
    decoder: ProbeEventDecoder,
    escape_deadline: Option<std::time::Instant>,
}

#[cfg(unix)]
impl UnixInput {
    fn open(context: TerminalContext, queries: QuerySet) -> io::Result<Self> {
        use rustix::fs::{Mode, OFlags, open};

        // stdin/stdout commonly share an open-file description. Reopen the
        // actual TTY instead of changing inherited flags (or dup'ing stdin),
        // so making input nonblocking cannot make output writes fail.
        let name = rustix::termios::ttyname(std::io::stdin(), Vec::new())?;
        let input = open(
            &name,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Self {
            input,
            replies: ReplyFilter::new(context, queries),
            decoder: ProbeEventDecoder::new(),
            escape_deadline: None,
        })
    }

    /// Reads currently available raw bytes. `WouldBlock` means AsyncFd should
    /// wait for another readiness notification.
    pub fn read_available(&mut self) -> io::Result<Vec<Event>> {
        self.read_limited(1024).map(|(_, events)| events)
    }

    fn read_limited(&mut self, limit: usize) -> io::Result<(usize, Vec<Event>)> {
        let mut buffer = [0_u8; 1024];
        let limit = limit.min(buffer.len());
        if limit == 0 {
            return Ok((0, Vec::new()));
        }
        loop {
            match rustix::io::read(&self.input, &mut buffer[..limit]) {
                Ok(0) => {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                Ok(read) => return Ok((read, self.feed(&buffer[..read]))),
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        {
            let (replies, decoder) = (&mut self.replies, &mut self.decoder);
            replies.push(bytes, |input| events.extend(decoder.push(input, true)));
        }
        self.escape_deadline =
            if self.replies.has_pending_control() || self.decoder.pending_escape() {
                std::time::Instant::now().checked_add(Duration::from_millis(25))
            } else {
                None
            };
        events
    }

    /// Returns a changed terminal fact snapshot learned after startup.
    pub fn take_terminal_info_update(&mut self) -> Option<TerminalInfo> {
        self.replies.take_update()
    }

    /// The one short timeout needed to distinguish Escape from Alt/CSI input.
    pub fn escape_deadline(&self) -> Option<std::time::Instant> {
        self.escape_deadline
    }

    /// Flushes only an expired Escape. Incomplete UTF-8, CSI, paste framing,
    /// and armed reply prefixes remain buffered for later bytes.
    pub fn flush_escape(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        if self.replies.has_pending_control() {
            let (replies, decoder) = (&mut self.replies, &mut self.decoder);
            replies.finish(|bytes| events.extend(decoder.push(bytes, false)));
        } else {
            events.extend(self.decoder.flush_escape());
        }
        self.escape_deadline = None;
        events
    }
}

#[cfg(unix)]
impl std::os::fd::AsRawFd for UnixInput {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(&self.input)
    }
}

/// Incrementally decodes bytes from the sole raw input reader.
///
/// The decoder keeps incomplete UTF-8, CSI, and bracketed-paste fragments until
/// another `push` supplies their continuation. The live path never calls
/// [`Self::finish`]; it only settles an isolated Escape with
/// [`Self::flush_escape`].
#[derive(Debug, Default)]
pub struct ProbeEventDecoder {
    termwiz: InputParser,
    pending: Vec<u8>,
    pasting: bool,
}

impl ProbeEventDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode_complete(bytes: &[u8]) -> Vec<Event> {
        let mut decoder = Self::new();
        let mut events = decoder.push(bytes, true);
        events.extend(decoder.finish());
        events
    }

    /// Adds raw bytes to the same parser used after startup. Incomplete UTF-8,
    /// CSI, paste, and Alt-key framing stay inside that parser until a later
    /// read supplies the continuation.
    pub fn push(&mut self, bytes: &[u8], maybe_more: bool) -> Vec<Event> {
        if self.pending.is_empty() && !bytes.contains(&b'\x1b') {
            let mut events = Vec::new();
            self.decode_with_termwiz(bytes, maybe_more, &mut events);
            return events;
        }
        self.pending.extend_from_slice(bytes);
        self.decode(maybe_more)
    }

    /// Flushes a finite injected stream. The live path must use
    /// [`Self::flush_escape`] instead so it never truncates a partial paste or
    /// UTF-8 sequence merely to resolve Escape ambiguity.
    pub fn finish(&mut self) -> Vec<Event> {
        self.decode(false)
    }

    fn pending_escape(&self) -> bool {
        !self.pasting && self.pending == b"\x1b"
    }

    fn flush_escape(&mut self) -> Vec<Event> {
        if self.pending_escape() {
            self.decode(false)
        } else {
            Vec::new()
        }
    }

    fn decode(&mut self, maybe_more: bool) -> Vec<Event> {
        let mut pending = std::mem::take(&mut self.pending);
        let mut events = Vec::new();
        let mut consumed = 0;
        while consumed < pending.len() {
            let input = &pending[consumed..];
            if self.pasting {
                if input[0] != b'\x1b' {
                    let len = input
                        .iter()
                        .position(|byte| *byte == b'\x1b')
                        .unwrap_or(input.len());
                    self.decode_with_termwiz(&input[..len], maybe_more, &mut events);
                    consumed += len;
                    continue;
                }
                if input.starts_with(PASTE_END) {
                    self.decode_with_termwiz(PASTE_END, maybe_more, &mut events);
                    consumed += PASTE_END.len();
                    self.pasting = false;
                    continue;
                }
                if PASTE_END.starts_with(input) {
                    if maybe_more {
                        break;
                    }
                    self.decode_with_termwiz(input, false, &mut events);
                    consumed = pending.len();
                    break;
                }
                let len = input[1..]
                    .iter()
                    .position(|byte| *byte == b'\x1b')
                    .map_or(input.len(), |offset| offset + 1);
                self.decode_with_termwiz(&input[..len], maybe_more, &mut events);
                consumed += len;
                continue;
            }
            if input.starts_with(PASTE_START) {
                self.decode_with_termwiz(PASTE_START, maybe_more, &mut events);
                consumed += PASTE_START.len();
                self.pasting = true;
                continue;
            }
            if PASTE_START.starts_with(input) {
                if maybe_more {
                    break;
                }
                self.decode_with_termwiz(input, false, &mut events);
                consumed = pending.len();
                break;
            }
            if input.starts_with(b"\x1b[M") {
                if input.len() < 6 {
                    if maybe_more {
                        break;
                    }
                    self.decode_with_termwiz(input, false, &mut events);
                    consumed = pending.len();
                    break;
                }
                if let Some(event) = parse_normal_mouse(&input[..6]) {
                    events.push(event);
                } else {
                    self.decode_with_termwiz(&input[..6], false, &mut events);
                }
                consumed += 6;
                continue;
            }
            if input.starts_with(b"\x1b[") {
                let Some(sequence_len) = csi_len(input) else {
                    if maybe_more && input.len() <= MAX_INPUT_ESCAPE_LEN {
                        break;
                    }
                    if maybe_more {
                        self.decode_with_termwiz(&input[..1], true, &mut events);
                        consumed += 1;
                        continue;
                    }
                    self.decode_with_termwiz(input, false, &mut events);
                    consumed = pending.len();
                    break;
                };
                let sequence = &input[..sequence_len];
                if let Some(event) = parse_csi_overlay(sequence) {
                    events.push(event);
                } else {
                    self.decode_with_termwiz(sequence, true, &mut events);
                }
                consumed += sequence_len;
                continue;
            }
            if input.starts_with(b"\x1bO") {
                if input.len() < 3 {
                    if maybe_more {
                        break;
                    }
                    self.decode_with_termwiz(input, false, &mut events);
                    consumed = pending.len();
                    break;
                }
                self.decode_with_termwiz(&input[..3], true, &mut events);
                consumed += 3;
                continue;
            }
            if input[0] == b'\x1b' {
                if input.len() == 1 && maybe_more {
                    break;
                }
                self.decode_with_termwiz(input, maybe_more, &mut events);
                consumed = pending.len();
                break;
            }

            let len = input
                .iter()
                .position(|byte| *byte == b'\x1b')
                .unwrap_or(input.len());
            self.decode_with_termwiz(&input[..len], maybe_more, &mut events);
            consumed += len;
        }
        pending.drain(..consumed);
        self.pending = pending;
        if !maybe_more {
            self.decode_with_termwiz(&[], false, &mut events);
        }
        events
    }

    fn decode_with_termwiz(&mut self, bytes: &[u8], maybe_more: bool, events: &mut Vec<Event>) {
        self.termwiz
            .parse(bytes, |event| events.push(termwiz_event(event)), maybe_more);
    }
}

fn csi_len(bytes: &[u8]) -> Option<usize> {
    let bytes = &bytes[..bytes.len().min(MAX_INPUT_ESCAPE_LEN)];
    if bytes.starts_with(b"\x1b[<") {
        return bytes
            .iter()
            .enumerate()
            .skip(3)
            .find(|(_, byte)| matches!(**byte, b'M' | b'm'))
            .map(|(index, _)| index + 1);
    }
    bytes
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, byte)| (0x40..=0x7e).contains(*byte))
        .map(|(index, _)| index + 1)
}

fn parse_csi_overlay(sequence: &[u8]) -> Option<Event> {
    match sequence {
        b"\x1b[I" => return Some(Event::FocusGained),
        b"\x1b[O" => return Some(Event::FocusLost),
        _ => {}
    }
    if sequence.ends_with(b"u") {
        return parse_kitty_key(sequence);
    }
    if sequence.starts_with(b"\x1b[<") {
        return parse_sgr_mouse(sequence);
    }
    if sequence.ends_with(b"M") && sequence.get(2).is_some_and(u8::is_ascii_digit) {
        return parse_rxvt_mouse(sequence);
    }
    parse_functional_key(sequence)
}

/// Kitty reports event kinds on legacy-shaped functional keys as well as CSI-u.
/// Feed these directly to the event stream: termwiz's legacy parser can retain
/// an unrecognized `;modifiers:event` frame and stall subsequent text input.
fn parse_functional_key(sequence: &[u8]) -> Option<Event> {
    let (&suffix, body) = sequence.strip_prefix(b"\x1b[")?.split_last()?;
    let body = std::str::from_utf8(body).ok()?;
    let mut fields = body.split(';');
    let number = match fields.next()? {
        "" => 1,
        value => value.parse::<u32>().ok()?,
    };
    let code = match (suffix, number) {
        (b'A', 1) => KeyCode::Up,
        (b'B', 1) => KeyCode::Down,
        (b'C', 1) => KeyCode::Right,
        (b'D', 1) => KeyCode::Left,
        (b'E', 1) | (b'~', 57427) => KeyCode::KeypadBegin,
        (b'H', 1) | (b'~', 1 | 7) => KeyCode::Home,
        (b'F', 1) | (b'~', 4 | 8) => KeyCode::End,
        (b'P', 1) | (b'~', 11) => KeyCode::F(1),
        (b'Q', 1) | (b'~', 12) => KeyCode::F(2),
        (b'R', 1) | (b'~', 13) => KeyCode::F(3),
        (b'S', 1) | (b'~', 14) => KeyCode::F(4),
        (b'~', 2) => KeyCode::Insert,
        (b'~', 3) => KeyCode::Delete,
        (b'~', 5) => KeyCode::PageUp,
        (b'~', 6) => KeyCode::PageDown,
        (b'~', 15) => KeyCode::F(5),
        (b'~', 17..=21) => KeyCode::F((number - 11) as u8),
        (b'~', 23..=24) => KeyCode::F((number - 12) as u8),
        _ => return None,
    };
    let event = kitty_key_event(code, fields.next().unwrap_or("1"))?;
    fields.next().is_none().then_some(Event::Key(event))
}

fn parse_kitty_key(sequence: &[u8]) -> Option<Event> {
    let body = std::str::from_utf8(sequence.strip_prefix(b"\x1b[")?.strip_suffix(b"u")?).ok()?;
    if !body
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'))
    {
        return None;
    }
    let mut fields = body.split(';');
    let mut codepoints = fields.next()?.split(':');
    let codepoint = codepoints.next()?.parse::<u32>().ok()?;
    let mut event = kitty_key_event(kitty_keycode(codepoint)?, fields.next().unwrap_or("1"))?;
    if event.modifiers.contains(KeyModifiers::SHIFT)
        && let Some(shifted) = codepoints
            .next()
            .and_then(|codepoint| codepoint.parse::<u32>().ok())
            .and_then(char::from_u32)
    {
        event.code = KeyCode::Char(shifted);
        event.modifiers.remove(KeyModifiers::SHIFT);
    }
    Some(Event::Key(event))
}

fn kitty_key_event(code: KeyCode, modifier_and_kind: &str) -> Option<KeyEvent> {
    let mut modifier_parts = modifier_and_kind.split(':');
    let modifier_mask = modifier_parts.next()?.parse::<u8>().ok()?;
    let kind = match modifier_parts.next() {
        None | Some("1") => KeyEventKind::Press,
        Some("2") => KeyEventKind::Repeat,
        Some("3") => KeyEventKind::Release,
        _ => return None,
    };
    if modifier_parts.next().is_some() {
        return None;
    }
    let mut state = KeyEventState::NONE;
    let bits = modifier_mask.saturating_sub(1);
    if bits & 64 != 0 {
        state |= KeyEventState::CAPS_LOCK;
    }
    if bits & 128 != 0 {
        state |= KeyEventState::NUM_LOCK;
    }
    Some(KeyEvent::new_with_kind_and_state(
        code,
        kitty_modifiers(modifier_mask),
        kind,
        state,
    ))
}

fn kitty_modifiers(mask: u8) -> KeyModifiers {
    let bits = mask.saturating_sub(1);
    let mut modifiers = KeyModifiers::NONE;
    if bits & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if bits & 8 != 0 {
        modifiers |= KeyModifiers::SUPER;
    }
    if bits & 16 != 0 {
        modifiers |= KeyModifiers::HYPER;
    }
    if bits & 32 != 0 {
        modifiers |= KeyModifiers::META;
    }
    modifiers
}

fn kitty_keycode(codepoint: u32) -> Option<KeyCode> {
    let keycode = match codepoint {
        57399..=57408 => KeyCode::Char(char::from_u32(codepoint - 57399 + u32::from(b'0'))?),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57414 => KeyCode::Enter,
        57415 => KeyCode::Char('='),
        57416 => KeyCode::Char(','),
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57427 => KeyCode::KeypadBegin,
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57361 => KeyCode::PrintScreen,
        57362 => KeyCode::Pause,
        57363 => KeyCode::Menu,
        57376..=57398 => KeyCode::F((codepoint - 57376 + 13) as u8),
        _ => match char::from_u32(codepoint)? {
            '\x1b' => KeyCode::Esc,
            '\r' => KeyCode::Enter,
            '\t' => KeyCode::Tab,
            '\x7f' => KeyCode::Backspace,
            character => KeyCode::Char(character),
        },
    };
    Some(keycode)
}

fn parse_normal_mouse(sequence: &[u8]) -> Option<Event> {
    let (kind, modifiers) = mouse_kind(sequence.get(3)?.checked_sub(32)?)?;
    Some(Event::Mouse(MouseEvent {
        kind,
        column: u16::from(sequence.get(4)?.saturating_sub(32)).checked_sub(1)?,
        row: u16::from(sequence.get(5)?.saturating_sub(32)).checked_sub(1)?,
        modifiers,
    }))
}

fn parse_rxvt_mouse(sequence: &[u8]) -> Option<Event> {
    let body = std::str::from_utf8(sequence.strip_prefix(b"\x1b[")?.strip_suffix(b"M")?).ok()?;
    let mut fields = body.split(';');
    let (kind, modifiers) = mouse_kind(fields.next()?.parse::<u8>().ok()?.checked_sub(32)?)?;
    Some(Event::Mouse(MouseEvent {
        kind,
        column: fields.next()?.parse::<u16>().ok()?.checked_sub(1)?,
        row: fields.next()?.parse::<u16>().ok()?.checked_sub(1)?,
        modifiers,
    }))
}

fn parse_sgr_mouse(sequence: &[u8]) -> Option<Event> {
    if sequence.len() < 4 {
        return None;
    }
    let released = sequence.last() == Some(&b'm');
    let body = std::str::from_utf8(&sequence[3..sequence.len() - 1]).ok()?;
    let mut fields = body.split(';');
    let (mut kind, modifiers) = mouse_kind(fields.next()?.parse().ok()?)?;
    if released && let MouseEventKind::Down(button) = kind {
        kind = MouseEventKind::Up(button);
    }
    Some(Event::Mouse(MouseEvent {
        kind,
        column: fields.next()?.parse::<u16>().ok()?.checked_sub(1)?,
        row: fields.next()?.parse::<u16>().ok()?.checked_sub(1)?,
        modifiers,
    }))
}

fn mouse_kind(code: u8) -> Option<(MouseEventKind, KeyModifiers)> {
    let button = (code & 0b11) | ((code & 0b1100_0000) >> 4);
    let dragging = code & 0b0010_0000 != 0;
    let kind = match (button, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3..=5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return None,
    };
    let mut modifiers = KeyModifiers::NONE;
    if code & 4 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if code & 8 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if code & 16 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some((kind, modifiers))
}

fn termwiz_event(event: TermwizEvent) -> Event {
    match event {
        TermwizEvent::Key(key) => Event::Key(KeyEvent::new(
            termwiz_keycode(key.key),
            termwiz_modifiers(key.modifiers),
        )),
        TermwizEvent::Paste(text) => Event::Paste(text),
        TermwizEvent::Mouse(mouse) => Event::Mouse(MouseEvent {
            kind: if mouse.mouse_buttons.is_empty() {
                MouseEventKind::Moved
            } else {
                MouseEventKind::Down(MouseButton::Left)
            },
            column: mouse.x.saturating_sub(1),
            row: mouse.y.saturating_sub(1),
            modifiers: termwiz_modifiers(mouse.modifiers),
        }),
        TermwizEvent::PixelMouse(mouse) => Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: mouse.x_pixels,
            row: mouse.y_pixels,
            modifiers: termwiz_modifiers(mouse.modifiers),
        }),
        TermwizEvent::Resized { cols, rows } => Event::Resize(
            u16::try_from(cols).unwrap_or(u16::MAX),
            u16::try_from(rows).unwrap_or(u16::MAX),
        ),
        TermwizEvent::Wake => Event::FocusGained,
    }
}

fn termwiz_modifiers(modifiers: TermwizMods) -> KeyModifiers {
    let mut out = KeyModifiers::NONE;
    if modifiers.intersects(TermwizMods::SHIFT | TermwizMods::LEFT_SHIFT | TermwizMods::RIGHT_SHIFT)
    {
        out |= KeyModifiers::SHIFT;
    }
    if modifiers.intersects(TermwizMods::ALT | TermwizMods::LEFT_ALT | TermwizMods::RIGHT_ALT) {
        out |= KeyModifiers::ALT;
    }
    if modifiers.intersects(TermwizMods::CTRL | TermwizMods::LEFT_CTRL | TermwizMods::RIGHT_CTRL) {
        out |= KeyModifiers::CONTROL;
    }
    if modifiers.contains(TermwizMods::SUPER) {
        out |= KeyModifiers::SUPER;
    }
    out
}

fn termwiz_keycode(key: TermwizKeyCode) -> KeyCode {
    match key {
        TermwizKeyCode::Char(character) => KeyCode::Char(character),
        TermwizKeyCode::Backspace => KeyCode::Backspace,
        TermwizKeyCode::Tab => KeyCode::Tab,
        TermwizKeyCode::Enter => KeyCode::Enter,
        TermwizKeyCode::Escape => KeyCode::Esc,
        TermwizKeyCode::LeftArrow | TermwizKeyCode::ApplicationLeftArrow => KeyCode::Left,
        TermwizKeyCode::RightArrow | TermwizKeyCode::ApplicationRightArrow => KeyCode::Right,
        TermwizKeyCode::UpArrow | TermwizKeyCode::ApplicationUpArrow => KeyCode::Up,
        TermwizKeyCode::DownArrow | TermwizKeyCode::ApplicationDownArrow => KeyCode::Down,
        TermwizKeyCode::PageUp | TermwizKeyCode::KeyPadPageUp => KeyCode::PageUp,
        TermwizKeyCode::PageDown | TermwizKeyCode::KeyPadPageDown => KeyCode::PageDown,
        TermwizKeyCode::Home | TermwizKeyCode::KeyPadHome => KeyCode::Home,
        TermwizKeyCode::End | TermwizKeyCode::KeyPadEnd => KeyCode::End,
        TermwizKeyCode::Insert => KeyCode::Insert,
        TermwizKeyCode::Delete => KeyCode::Delete,
        TermwizKeyCode::Function(number) => KeyCode::F(number),
        TermwizKeyCode::CapsLock => KeyCode::CapsLock,
        TermwizKeyCode::NumLock => KeyCode::NumLock,
        TermwizKeyCode::ScrollLock => KeyCode::ScrollLock,
        TermwizKeyCode::Pause => KeyCode::Pause,
        TermwizKeyCode::PrintScreen => KeyCode::PrintScreen,
        TermwizKeyCode::Menu | TermwizKeyCode::Applications => KeyCode::Menu,
        TermwizKeyCode::KeyPadBegin => KeyCode::KeypadBegin,
        _ => KeyCode::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::HostOs;

    #[derive(Default)]
    struct FakeIo {
        writes: Vec<Vec<u8>>,
        reply: Vec<u8>,
        partial_error: bool,
    }

    impl StartupProbeIo for FakeIo {
        fn write_query(&mut self, query: &[u8]) -> io::Result<()> {
            self.writes.push(query.to_vec());
            Ok(())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn read_bounded(&mut self, _: StartupProbeConfig) -> ProbeRead {
            let bytes = std::mem::take(&mut self.reply);
            if self.partial_error {
                ProbeRead::partial(bytes, io::Error::other("simulated PTY read error"))
            } else {
                ProbeRead::complete(bytes)
            }
        }
    }

    fn context(brand: TerminalName) -> TerminalContext {
        TerminalContext {
            brand,
            multiplexer: MultiplexerKind::Undetected,
            host: HostOs::Other,
        }
    }

    #[test]
    fn controlled_io_consumes_only_valid_replies_and_replays_typeahead() {
        let mut io = FakeIo {
            reply: b"hi\xe7\x8c\xab\x1b]11;rgb:0000/0000/0000\x07\x1b[>0;2401;1c\x1b[A".to_vec(),
            ..FakeIo::default()
        };
        let mut outcome = StartupProbe::for_context(context(TerminalName::Alacritty)).run(&mut io);

        assert_eq!(io.writes, vec![OSC11_QUERY.to_vec(), da2::QUERY.to_vec()]);
        assert_eq!(
            outcome.info.background.unwrap().appearance(),
            crate::appearance::SystemAppearance::Dark
        );
        assert_eq!(outcome.info.da2.unwrap().packed(), 2401);
        assert_eq!(
            outcome.take_events(),
            vec![
                Event::Key(KeyCode::Char('h').into()),
                Event::Key(KeyCode::Char('i').into()),
                Event::Key(KeyCode::Char('猫').into()),
                Event::Key(KeyCode::Up.into()),
            ]
        );
    }

    #[test]
    fn partial_read_error_replays_observed_unicode_input() {
        let mut io = FakeIo {
            reply: "猫".as_bytes().to_vec(),
            partial_error: true,
            ..FakeIo::default()
        };
        let mut outcome = StartupProbe::for_context(context(TerminalName::Kitty)).run(&mut io);

        assert_eq!(
            outcome.take_events(),
            vec![Event::Key(KeyCode::Char('猫').into())]
        );
    }

    #[test]
    fn timed_out_probe_replays_typeahead() {
        let mut io = FakeIo {
            reply: b"typed".to_vec(),
            ..FakeIo::default()
        };
        let mut outcome = StartupProbe::for_context(context(TerminalName::Kitty)).run(&mut io);

        assert_eq!(
            outcome.take_events(),
            vec![
                Event::Key(KeyCode::Char('t').into()),
                Event::Key(KeyCode::Char('y').into()),
                Event::Key(KeyCode::Char('p').into()),
                Event::Key(KeyCode::Char('e').into()),
                Event::Key(KeyCode::Char('d').into()),
            ]
        );
    }
    #[test]
    fn fragmented_replies_survive_the_startup_live_handoff() {
        let queries = QuerySet {
            osc11: true,
            da2: true,
            xtversion: true,
            cursor: false,
        };
        let mut filter = ReplyFilter::new(context(TerminalName::Alacritty), queries);
        let mut decoder = ProbeEventDecoder::new();
        let mut events = Vec::new();

        for fragment in [
            b"a\x1b]11;rgb:0000/".as_slice(),
            b"0000/0000\x07\x1b[>0;24",
            b"01;1c\x1bP>|Kitty ",
            b"0.35.2\x1b\\Z",
        ] {
            filter.push(fragment, |bytes| events.extend(decoder.push(bytes, true)));
        }

        assert_eq!(
            filter.info.background.unwrap().appearance(),
            crate::appearance::SystemAppearance::Dark
        );
        assert_eq!(filter.info.da2.unwrap().packed(), 2401);
        assert_eq!(filter.info.xtversion.unwrap().as_str(), "Kitty 0.35.2");
        assert_eq!(
            events,
            vec![
                Event::Key(KeyCode::Char('a').into()),
                Event::Key(KeyCode::Char('Z').into()),
            ]
        );
    }

    #[test]
    fn timeout_keeps_unknown_and_mux_input_unread() {
        let mut io = FakeIo {
            reply: b"typed".to_vec(),
            ..FakeIo::default()
        };
        let mut muxed = context(TerminalName::Kitty);
        muxed.multiplexer = MultiplexerKind::Tmux;
        let mut outcome = StartupProbe::for_context(muxed).run(&mut io);
        assert!(io.writes.is_empty());
        assert_eq!(io.reply, b"typed");
        assert!(outcome.take_events().is_empty());

        let mut unknown_io = FakeIo::default();
        let _ = StartupProbe::for_context(context(TerminalName::Unknown)).run(&mut unknown_io);
        assert!(unknown_io.writes.is_empty());
    }

    #[test]
    fn decoder_reassembles_utf8_paste_mouse_and_kitty_release() {
        let mut decoder = ProbeEventDecoder::new();
        assert!(decoder.push(b"\xe7", true).is_empty());
        assert!(decoder.push(b"\x8c", true).is_empty());
        assert_eq!(
            decoder.push(b"\xab\x1b[200~first\n", true),
            vec![Event::Key(KeyCode::Char('猫').into())]
        );
        assert_eq!(
            decoder.push(b"second\x1b[201~\x1b[<0;3;4M\x1b[13;1:3u\x1b[I\x1b[O", true),
            vec![
                Event::Paste("first\nsecond".to_owned()),
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 2,
                    row: 3,
                    modifiers: KeyModifiers::NONE,
                }),
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                Event::FocusGained,
                Event::FocusLost,
            ]
        );
    }

    #[test]
    fn decoder_reassembles_split_paste_markers() {
        let mut decoder = ProbeEventDecoder::new();

        assert!(decoder.push(b"\x1b[2", true).is_empty());
        assert!(decoder.push(b"00~first\nsecond\x1b[20", true).is_empty());
        assert_eq!(
            decoder.push(b"1~", true),
            vec![Event::Paste("first\nsecond".to_owned())]
        );
    }

    #[test]
    fn pasted_control_bytes_are_opaque_across_split_frames() {
        let queries = QuerySet {
            osc11: true,
            da2: true,
            xtversion: true,
            cursor: true,
        };
        let mut filter = ReplyFilter::new(context(TerminalName::Alacritty), queries);
        let mut decoder = ProbeEventDecoder::new();
        let mut events = Vec::new();
        let payload = concat!(
            "first\n",
            "\x1b]11;rgb:0000/0000/0000\x07",
            "\x1b[>0;2401;1c",
            "\x1b[3;5R",
            "\x1bP>|Kitty 0.35.2\x1b\\",
            "\x1b[13;1:3u",
            "\x1b[<0;3;4M",
            "last",
        );

        for fragment in [
            b"\x1b[2".as_slice(),
            b"00~first\n\x1b]1",
            b"1;rgb:0000/0000/0000\x07\x1b[>0;24",
            b"01;1c\x1b[3;5R\x1bP>|Kitty 0.35.2\x1b\\\x1b[13;1:3u\x1b[<0;3;4Mlast\x1b[20",
            b"1~",
        ] {
            filter.push(fragment, |bytes| events.extend(decoder.push(bytes, true)));
        }

        assert_eq!(events, vec![Event::Paste(payload.to_owned())]);
        assert!(filter.info.background.is_none());
        assert!(filter.info.da2.is_none());
        assert!(filter.info.xtversion.is_none());
        assert!(filter.cursor_position.is_none());
        assert!(filter.take_update().is_none());
    }

    #[test]
    fn functional_key_releases_leave_following_text_decodable() {
        for (sequence, code, kind, modifiers) in [
            (
                b"\x1b[1;1:3A".as_slice(),
                KeyCode::Up,
                KeyEventKind::Release,
                KeyModifiers::NONE,
            ),
            (
                b"\x1b[6;2:2~",
                KeyCode::PageDown,
                KeyEventKind::Repeat,
                KeyModifiers::SHIFT,
            ),
            (
                b"\x1b[1;5:3H",
                KeyCode::Home,
                KeyEventKind::Release,
                KeyModifiers::CONTROL,
            ),
            (
                b"\x1b[13;1:3~",
                KeyCode::F(3),
                KeyEventKind::Release,
                KeyModifiers::NONE,
            ),
        ] {
            let mut decoder = ProbeEventDecoder::new();
            assert_eq!(
                decoder.push(sequence, true),
                vec![Event::Key(KeyEvent::new_with_kind(code, modifiers, kind))]
            );
            assert_eq!(
                decoder.push("hst界".as_bytes(), true),
                vec![
                    Event::Key(KeyCode::Char('h').into()),
                    Event::Key(KeyCode::Char('s').into()),
                    Event::Key(KeyCode::Char('t').into()),
                    Event::Key(KeyCode::Char('界').into()),
                ]
            );
        }
    }

    #[test]
    fn lone_escape_flushes_after_an_armed_reply_prefix() {
        let queries = QuerySet {
            osc11: true,
            da2: false,
            xtversion: false,
            cursor: false,
        };
        let mut filter = ReplyFilter::new(context(TerminalName::Kitty), queries);
        let mut decoder = ProbeEventDecoder::new();
        let mut events = Vec::new();

        filter.push(b"\x1b", |bytes| events.extend(decoder.push(bytes, true)));
        assert!(events.is_empty());
        assert!(filter.has_pending_control());
        filter.finish(|bytes| events.extend(decoder.push(bytes, false)));

        assert_eq!(events, vec![Event::Key(KeyCode::Esc.into())]);
    }

    #[test]
    fn csi_scanner_rejects_overlong_controls() {
        let mut overlong = vec![b'\x1b', b'['];
        overlong.extend(std::iter::repeat_n(b'1', MAX_INPUT_ESCAPE_LEN));
        overlong.push(b'A');

        assert_eq!(csi_len(&overlong), None);
        assert_eq!(csi_len(b"\x1b[13;1:3u"), Some(9));
    }

    #[test]
    fn cursor_reply_is_consumed_and_exposes_the_inline_origin() {
        let mut io = FakeIo {
            reply: b"\x1b[3;5R".to_vec(),
            ..FakeIo::default()
        };
        let mut outcome = StartupProbe::for_context(context(TerminalName::Unknown))
            .with_cursor_position()
            .run(&mut io);

        assert_eq!(io.writes, vec![CURSOR_POSITION_QUERY.to_vec()]);
        assert_eq!(outcome.cursor_position, Some(Position::new(4, 2)));
        assert!(outcome.take_events().is_empty());
    }

    #[test]
    fn cursor_probe_does_not_delay_live_keyboard_frames() {
        for answered in [false, true] {
            let mut filter = ReplyFilter::new(
                context(TerminalName::Kitty),
                QuerySet {
                    osc11: false,
                    da2: false,
                    xtversion: false,
                    cursor: true,
                },
            );
            let mut decoder = ProbeEventDecoder::new();
            let mut events = Vec::new();
            if answered {
                filter.push(b"\x1b[3;5R", |bytes| {
                    events.extend(decoder.push(bytes, true))
                });
            }
            for (frame, expected) in [
                (
                    b"\x1b[114;5u".as_slice(),
                    KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                ),
                (
                    b"\x1b[114;5:3u",
                    KeyEvent::new_with_kind(
                        KeyCode::Char('r'),
                        KeyModifiers::CONTROL,
                        KeyEventKind::Release,
                    ),
                ),
                (b"j", KeyEvent::from(KeyCode::Char('j'))),
                (b"k", KeyEvent::from(KeyCode::Char('k'))),
                (b"\r", KeyEvent::from(KeyCode::Enter)),
                (b"\x1b[27u", KeyEvent::from(KeyCode::Esc)),
                (
                    b"\x1b[27;1:3u",
                    KeyEvent::new_with_kind(
                        KeyCode::Esc,
                        KeyModifiers::NONE,
                        KeyEventKind::Release,
                    ),
                ),
                (
                    b"\x1b[1;2A",
                    KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
                ),
            ] {
                filter.push(frame, |bytes| events.extend(decoder.push(bytes, true)));
                assert_eq!(
                    events,
                    vec![Event::Key(expected)],
                    "answered={answered}, frame={frame:?}"
                );
                events.clear();
            }
            if answered {
                let mut raw = Vec::new();
                filter.push(b"\x1b[1;2R", |bytes| raw.extend_from_slice(bytes));
                assert_eq!(
                    raw, b"\x1b[1;2R",
                    "a completed probe must not swallow Shift-F3"
                );
            }
        }
    }

    #[test]
    fn xtversion_reply_is_removed_without_querying_unknown_brands() {
        let mut io = FakeIo {
            reply: b"\x1bP>|Kitty 0.35.2\x1b\\".to_vec(),
            ..FakeIo::default()
        };
        let outcome = StartupProbe::for_context(context(TerminalName::Kitty)).run(&mut io);
        assert_eq!(
            io.writes,
            vec![OSC11_QUERY.to_vec(), xtversion::QUERY.to_vec()]
        );
        assert_eq!(outcome.info.xtversion.unwrap().as_str(), "Kitty 0.35.2");
    }
}
