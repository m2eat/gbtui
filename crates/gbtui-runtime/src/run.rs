//! Live TTY event loop.

use std::io::{self, stdout};
use std::ops::{Deref, DerefMut};
use std::time::Instant;

use crossterm::Command;
use crossterm::ExecutableCommand;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, EventStream, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use futures_util::StreamExt;
use gbtui_core::{
    Theme, ThemeAppearance, ThemeContext, ThemeDefinition, ThemeKind, ThemeResolver, color_support,
};
use gbtui_terminal::kitty::{clear_pushed_kitty_flags, set_pushed_kitty_flags};
use gbtui_terminal::{Terminal, TerminalInfo, flags_for_context, terminal_context};
use ratatui::backend::CrosstermBackend;
use ratatui::style::Color;

#[cfg(feature = "images")]
use gbtui_terminal::image::{ImageScene, graphics_protocol};
#[cfg(unix)]
use gbtui_terminal::{StartupProbe, UnixInput};
use ratatui::{TerminalOptions, Viewport};
#[cfg(unix)]
use tokio::io::{Interest, unix::AsyncFd};
use tokio::sync::mpsc;

use crate::app::{App, Outcome};
use crate::pump::Pump;

/// Theme selection performed after bounded terminal probing.
///
/// [`RuntimeTheme::Kind`] and [`RuntimeTheme::Definition`] resolve against
/// terminal context for their entire lifetime. An explicit [`Theme`] always
/// passes through unchanged.
#[derive(Clone, Debug)]
pub enum RuntimeTheme {
    Explicit(Theme),
    Kind(ThemeKind),
    Definition(ThemeDefinition),
}

impl From<Theme> for RuntimeTheme {
    fn from(theme: Theme) -> Self {
        Self::Explicit(theme)
    }
}

impl From<ThemeKind> for RuntimeTheme {
    fn from(kind: ThemeKind) -> Self {
        Self::Kind(kind)
    }
}

impl From<ThemeDefinition> for RuntimeTheme {
    fn from(definition: ThemeDefinition) -> Self {
        Self::Definition(definition)
    }
}

impl RuntimeTheme {
    fn into_resolver(self, context: ThemeContext) -> ThemeResolver {
        match self {
            Self::Explicit(theme) => {
                let mut resolver = ThemeResolver::explicit(theme);
                let _ = resolver.set_context(context);
                resolver
            }
            Self::Kind(kind) => ThemeResolver::new(ThemeDefinition::from(kind), context),
            Self::Definition(definition) => ThemeResolver::new(definition, context),
        }
    }
}

fn theme_context(info: &TerminalInfo) -> ThemeContext {
    ThemeContext {
        appearance: info.appearance().map(|appearance| match appearance {
            gbtui_terminal::SystemAppearance::Light => ThemeAppearance::Light,
            gbtui_terminal::SystemAppearance::Dark => ThemeAppearance::Dark,
        }),
        color_level: color_support::get(),
        terminal_foreground: None,
        terminal_background: info
            .background
            .map(|color| Color::Rgb(color.red, color.green, color.blue)),
    }
}

/// Terminal features enabled by this runtime invocation.
///
/// Each flag is set only after its command succeeds, so cleanup never undoes
/// terminal state this invocation did not acquire.
#[derive(Default)]
struct TerminalProtocol {
    alternate_screen: bool,
    bracketed_paste: bool,
    focus_change: bool,
    mouse_capture: bool,
    kitty_flags: Option<KeyboardEnhancementFlags>,
}

impl TerminalProtocol {
    fn activate<W: io::Write>(
        &mut self,
        output: &mut W,
        alternate_screen: bool,
        kitty_flags: KeyboardEnhancementFlags,
    ) -> io::Result<()> {
        if alternate_screen {
            output.execute(EnterAlternateScreen)?;
            self.alternate_screen = true;
        }

        output.execute(EnableBracketedPaste)?;
        self.bracketed_paste = true;
        output.execute(EnableFocusChange)?;
        self.focus_change = true;
        output.execute(EnableMouseCapture)?;
        self.mouse_capture = true;

        if !kitty_flags.is_empty() {
            output.execute(PushKeyboardEnhancementFlags(kitty_flags))?;
            self.kitty_flags = Some(kitty_flags);
            set_pushed_kitty_flags(kitty_flags);
        }
        Ok(())
    }

    fn restore<W: io::Write>(&mut self, output: &mut W) -> io::Result<()> {
        let mut first_error = None;

        if let Some(flags) = self.kitty_flags {
            match output.execute(PopKeyboardEnhancementFlags) {
                Ok(_) => {
                    clear_pushed_kitty_flags(flags);
                    self.kitty_flags = None;
                }
                Err(error) => first_error = Some(error),
            }
        }
        restore_command(
            output,
            &mut self.mouse_capture,
            DisableMouseCapture,
            &mut first_error,
        );
        restore_command(
            output,
            &mut self.focus_change,
            DisableFocusChange,
            &mut first_error,
        );
        restore_command(
            output,
            &mut self.bracketed_paste,
            DisableBracketedPaste,
            &mut first_error,
        );
        restore_command(
            output,
            &mut self.alternate_screen,
            LeaveAlternateScreen,
            &mut first_error,
        );

        first_error.map_or(Ok(()), Err)
    }
}

fn restore_command<W: io::Write, C: Command>(
    output: &mut W,
    active: &mut bool,
    command: C,
    first_error: &mut Option<io::Error>,
) {
    if !*active {
        return;
    }

    match output.execute(command) {
        Ok(_) => *active = false,
        Err(error) if first_error.is_none() => *first_error = Some(error),
        Err(_) => {}
    }
}

/// Restores terminal state even when the loop future is cancelled or unwinds.
#[derive(Default)]
struct TerminalSession {
    raw_mode: bool,
    protocol: TerminalProtocol,
}

impl TerminalSession {
    fn enable_raw(&mut self) -> io::Result<()> {
        if !crossterm::terminal::is_raw_mode_enabled()? {
            enable_raw_mode()?;
            self.raw_mode = true;
        }
        Ok(())
    }

    fn activate<W: io::Write>(
        &mut self,
        output: &mut W,
        alternate_screen: bool,
        kitty_flags: KeyboardEnhancementFlags,
    ) -> io::Result<()> {
        self.protocol
            .activate(output, alternate_screen, kitty_flags)
    }

    fn restore<W: io::Write>(&mut self, output: &mut W) -> io::Result<()> {
        let protocol_result = self.protocol.restore(output);
        let raw_result = if self.raw_mode {
            let result = disable_raw_mode();
            if result.is_ok() {
                self.raw_mode = false;
            }
            result
        } else {
            Ok(())
        };

        match (protocol_result, raw_result) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let mut output = stdout();
        let _ = self.restore(&mut output);
    }
}

enum IncomingEvents {
    One(Event),
    Batch(Vec<Event>),
}

/// One and only one source may own stdin after raw mode is enabled.
enum LiveInput<'a> {
    Crossterm(&'a mut EventStream),
    #[cfg(unix)]
    Unix(&'a mut AsyncFd<UnixInput>),
}

impl LiveInput<'_> {
    async fn next_events(&mut self) -> io::Result<Option<IncomingEvents>> {
        match self {
            Self::Crossterm(events) => match events.next().await {
                Some(event) => event.map(|event| Some(IncomingEvents::One(event))),
                None => Ok(None),
            },
            #[cfg(unix)]
            Self::Unix(input) => input
                .async_io_mut(Interest::READABLE, UnixInput::read_available)
                .await
                .map(|events| Some(IncomingEvents::Batch(events))),
        }
    }

    fn terminal_info_update(&mut self) -> Option<TerminalInfo> {
        match self {
            Self::Crossterm(_) => None,
            #[cfg(unix)]
            Self::Unix(input) => input.get_mut().take_terminal_info_update(),
        }
    }

    fn escape_deadline(&self) -> Option<Instant> {
        match self {
            Self::Crossterm(_) => None,
            #[cfg(unix)]
            Self::Unix(input) => input.get_ref().escape_deadline(),
        }
    }

    fn flush_escape(&mut self) -> Vec<Event> {
        match self {
            Self::Crossterm(_) => Vec::new(),
            #[cfg(unix)]
            Self::Unix(input) => input.get_mut().flush_escape(),
        }
    }

    #[cfg(unix)]
    fn has_unix_reader(&self) -> bool {
        matches!(self, Self::Unix(_))
    }
}

struct TerminalStartup {
    info: TerminalInfo,
    events: Vec<Event>,
    #[cfg(unix)]
    input: Option<AsyncFd<UnixInput>>,
    cursor_position: Option<ratatui::layout::Position>,
}

/// Owns the live raw-output path and guarantees protocol cleanup before the
/// terminal session unwinds, including cancellation and panic unwinding.
struct LivePump<A: App, B: ratatui::backend::Backend + io::Write> {
    pump: Pump<A, B>,
    inline: bool,
    #[cfg(feature = "images")]
    images: ImageScene,
    #[cfg(feature = "images")]
    images_supported: bool,
}

impl<A: App, B: ratatui::backend::Backend + io::Write> LivePump<A, B> {
    fn new(pump: Pump<A, B>, inline: bool) -> Self {
        #[cfg(feature = "images")]
        let images = ImageScene::new(graphics_protocol());
        #[cfg(feature = "images")]
        let images_supported = images.supports_images();
        Self {
            pump,
            inline,
            #[cfg(feature = "images")]
            images,
            #[cfg(feature = "images")]
            images_supported,
        }
    }

    fn paint(&mut self) -> io::Result<()> {
        #[cfg(feature = "images")]
        {
            self.pump
                .paint_live_with_images(&mut self.images, self.images_supported)
        }
        #[cfg(not(feature = "images"))]
        self.pump.paint_live()
    }

    fn rerender_inline_history(&mut self) -> io::Result<()> {
        self.pump.rerender_inline_history()?;
        #[cfg(feature = "images")]
        {
            self.images.clear(self.pump.terminal_mut().backend_mut())?;
            io::Write::flush(self.pump.terminal_mut().backend_mut())?;
        }
        Ok(())
    }
}

impl<A: App, B: ratatui::backend::Backend + io::Write> Deref for LivePump<A, B> {
    type Target = Pump<A, B>;

    fn deref(&self) -> &Self::Target {
        &self.pump
    }
}

impl<A: App, B: ratatui::backend::Backend + io::Write> DerefMut for LivePump<A, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.pump
    }
}

impl<A: App, B: ratatui::backend::Backend + io::Write> Drop for LivePump<A, B> {
    fn drop(&mut self) {
        #[cfg(feature = "images")]
        {
            let _ = self.images.clear(self.pump.terminal_mut().backend_mut());
        }
        if self.inline {
            let _ = self.pump.terminal_mut().clear();
        }
        let _ = io::Write::flush(self.pump.terminal_mut().backend_mut());
    }
}

/// Blocking wrapper around [`run`].
pub fn run_blocking<A: App>(app: A, theme: impl Into<RuntimeTheme>) -> io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run(app, theme, None))
}

/// Async event loop on the alternate screen.
pub async fn run<A: App>(
    app: A,
    theme: impl Into<RuntimeTheme>,
    mut msgs: Option<mpsc::UnboundedReceiver<A::Msg>>,
) -> io::Result<()> {
    let mut session = TerminalSession::default();
    let startup = begin_terminal_session(&mut session, true)?;
    let resolver = theme.into().into_resolver(theme_context(&startup.info));

    let backend = CrosstermBackend::new(stdout());
    let terminal = Terminal::new(backend)?;
    let pump = Pump::with_resolver(app, terminal, resolver, Instant::now());
    let mut live = LivePump::new(pump, false);
    run_inner(&mut live, &mut msgs, startup).await
}

/// Blocking inline viewport: history goes to native scrollback, UI stays
/// at the bottom (`height` rows).
pub fn run_inline_blocking<A: App>(
    app: A,
    theme: impl Into<RuntimeTheme>,
    height: u16,
) -> io::Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(run_inline(app, theme, height, None))
}

/// Async inline viewport loop. No alternate screen.
pub async fn run_inline<A: App>(
    app: A,
    theme: impl Into<RuntimeTheme>,
    height: u16,
    mut msgs: Option<mpsc::UnboundedReceiver<A::Msg>>,
) -> io::Result<()> {
    let mut session = TerminalSession::default();
    let startup = begin_terminal_session(&mut session, false)?;
    let resolver = theme.into().into_resolver(theme_context(&startup.info));
    let options = TerminalOptions {
        viewport: Viewport::Inline(height.max(1)),
    };
    let backend = CrosstermBackend::new(stdout());
    #[cfg(unix)]
    let terminal = match startup.cursor_position {
        Some(cursor) => Terminal::with_options_at_cursor(backend, options, cursor)?,
        None => Terminal::with_options(backend, options)?,
    };
    #[cfg(not(unix))]
    let terminal = Terminal::with_options(backend, options)?;

    let mut live = LivePump::new(
        Pump::with_resolver(app, terminal, resolver, Instant::now()),
        true,
    );
    run_inner(&mut live, &mut msgs, startup).await
}

/// Establishes the only input-reader ordering allowed during startup:
/// raw mode, bounded probe, keyboard flags, then the single live input source.
fn begin_terminal_session(
    session: &mut TerminalSession,
    alternate_screen: bool,
) -> io::Result<TerminalStartup> {
    session.enable_raw()?;
    let context = terminal_context();
    #[cfg(unix)]
    let probe = if alternate_screen {
        StartupProbe::for_context(context)
    } else {
        StartupProbe::for_context(context).with_cursor_position()
    };

    #[cfg(unix)]
    let (info, events, cursor_position, unix_input) = match probe.run_live()? {
        Some(mut probe) => {
            let info = probe.info.clone();
            let events = probe.take_events();
            let cursor_position = probe.cursor_position;
            (info, events, cursor_position, Some(probe.into_input()))
        }
        None => (TerminalInfo::from_context(context), Vec::new(), None, None),
    };
    #[cfg(not(unix))]
    let (info, events, cursor_position) = (TerminalInfo::from_context(context), Vec::new(), None);

    let da2_packed = info.da2.map(|version| version.packed());
    let mut output = stdout();
    session.activate(
        &mut output,
        alternate_screen,
        flags_for_context(&context, da2_packed),
    )?;

    #[cfg(unix)]
    let input = unix_input.map(AsyncFd::new).transpose()?;
    Ok(TerminalStartup {
        info,
        events,
        #[cfg(unix)]
        input,
        cursor_position,
    })
}

async fn run_inner<A: App, B: ratatui::backend::Backend + io::Write>(
    pump: &mut LivePump<A, B>,
    msgs: &mut Option<mpsc::UnboundedReceiver<A::Msg>>,
    mut startup: TerminalStartup,
) -> io::Result<()> {
    let context = theme_context(&startup.info);
    let mut initial = pump.handle_terminal_info(startup.info, context);
    if initial == Outcome::Quit {
        return Ok(());
    }
    for event in std::mem::take(&mut startup.events) {
        initial = initial.merge(pump.drive(Instant::now(), event));
        if initial == Outcome::Quit {
            return Ok(());
        }
    }
    if apply_initial_outcome(pump, initial)? {
        return Ok(());
    }

    #[cfg(unix)]
    let mut legacy = startup.input.is_none().then(EventStream::new);
    #[cfg(unix)]
    let mut input = match startup.input.as_mut() {
        Some(input) => LiveInput::Unix(input),
        None => LiveInput::Crossterm(legacy.as_mut().expect("fallback input stream")),
    };
    #[cfg(not(unix))]
    let mut legacy = EventStream::new();
    #[cfg(not(unix))]
    let mut input = LiveInput::Crossterm(&mut legacy);
    let mut resize_signal = make_resize_signal(&input)?;
    pump.paint()?;
    loop {
        let now = Instant::now();
        let tick_at = pump.tick_deadline(now);
        let resize_at = pump.resize_deadline();
        let escape_at = input.escape_deadline();

        tokio::select! {
            input_events = input.next_events() => {
                let Some(input_events) = input_events? else { break; };
                let mut outcome = Outcome::Unchanged;
                match input_events {
                    IncomingEvents::One(event) => {
                        outcome = outcome.merge(pump.drive(Instant::now(), event));
                    }
                    IncomingEvents::Batch(events) => {
                        for event in events {
                            outcome = outcome.merge(pump.drive(Instant::now(), event));
                            if outcome == Outcome::Quit {
                                return Ok(());
                            }
                        }
                    }
                }
                if let Some(info) = input.terminal_info_update() {
                    let context = theme_context(&info);
                    outcome = outcome.merge(pump.handle_terminal_info(info, context));
                }
                if apply_live_outcome(pump, outcome)? {
                    break;
                }
            }
            _ = sleep_opt(escape_at) => {
                let mut outcome = Outcome::Unchanged;
                for event in input.flush_escape() {
                    outcome = outcome.merge(pump.drive(Instant::now(), event));
                    if outcome == Outcome::Quit {
                        return Ok(());
                    }
                }
                if apply_live_outcome(pump, outcome)? {
                    break;
                }
            }
            resize = next_resize(&mut resize_signal) => {
                let outcome = pump.drive(Instant::now(), resize?);
                if apply_live_outcome(pump, outcome)? {
                    break;
                }
            }
            _ = sleep_opt(tick_at) => {
                match pump.tick(Instant::now()) {
                    Outcome::Quit => break,
                    Outcome::Changed | Outcome::Unchanged => {
                        pump.commit_inline()?;
                        pump.paint()?;
                    }
                }
            }
            _ = sleep_opt(resize_at) => {
                match pump.flush_resize(Instant::now()) {
                    Outcome::Quit => break,
                    Outcome::Changed => {
                        pump.rerender_inline_history()?;
                        pump.paint()?;
                    }
                    Outcome::Unchanged => {}
                }
            }
            msg = recv_opt(msgs) => {
                let Some(msg) = msg else { continue; };
                let outcome = pump.drive_msg(msg);
                if apply_live_outcome(pump, outcome)? {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn apply_initial_outcome<A: App, B: ratatui::backend::Backend + io::Write>(
    pump: &mut LivePump<A, B>,
    outcome: Outcome,
) -> io::Result<bool> {
    match outcome {
        Outcome::Quit => Ok(true),
        Outcome::Changed => {
            pump.commit_inline()?;
            Ok(false)
        }
        Outcome::Unchanged => Ok(false),
    }
}

fn apply_live_outcome<A: App, B: ratatui::backend::Backend + io::Write>(
    pump: &mut LivePump<A, B>,
    outcome: Outcome,
) -> io::Result<bool> {
    match outcome {
        Outcome::Quit => Ok(true),
        Outcome::Changed => {
            pump.commit_inline()?;
            pump.paint()?;
            Ok(false)
        }
        Outcome::Unchanged => Ok(false),
    }
}

#[cfg(unix)]
fn make_resize_signal(input: &LiveInput<'_>) -> io::Result<Option<tokio::signal::unix::Signal>> {
    input
        .has_unix_reader()
        .then(|| tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()))
        .transpose()
}

#[cfg(not(unix))]
fn make_resize_signal(_: &LiveInput) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn next_resize(signal: &mut Option<tokio::signal::unix::Signal>) -> io::Result<Event> {
    match signal.as_mut() {
        Some(signal) => {
            if signal.recv().await.is_none() {
                return std::future::pending().await;
            }
            let (width, height) = crossterm::terminal::size()?;
            Ok(Event::Resize(width, height))
        }
        None => std::future::pending().await,
    }
}

#[cfg(not(unix))]
async fn next_resize(_: &mut ()) -> io::Result<Event> {
    std::future::pending().await
}

async fn sleep_opt(at: Option<Instant>) {
    match at {
        Some(deadline) => {
            let now = Instant::now();
            if deadline <= now {
                return;
            }
            tokio::time::sleep(deadline - now).await;
        }
        None => std::future::pending::<()>().await,
    }
}

async fn recv_opt<M>(rx: &mut Option<mpsc::UnboundedReceiver<M>>) -> Option<M> {
    let message = match rx.as_mut() {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    };
    if message.is_none() {
        *rx = None;
    }
    message
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};
    use std::task::Poll;

    use crossterm::event::KeyboardEnhancementFlags;
    use gbtui_core::{
        ColorLevel, Theme, ThemeAppearance, ThemeContext, ThemeDefinition, ThemeKind,
    };
    use gbtui_terminal::kitty::{kitty_flags_pushed, pushed_kitty_flags};
    use tokio::sync::mpsc;

    use super::{RuntimeTheme, TerminalProtocol, recv_opt};

    #[tokio::test]
    async fn closed_receiver_detaches_after_buffered_messages() {
        let (tx, receiver) = mpsc::unbounded_channel();
        tx.send(1).unwrap();
        drop(tx);
        let mut receiver = Some(receiver);

        assert_eq!(recv_opt(&mut receiver).await, Some(1));
        assert!(receiver.is_some());
        assert_eq!(recv_opt(&mut receiver).await, None);
        assert!(receiver.is_none());

        let mut pending = std::pin::pin!(recv_opt(&mut receiver));
        let result = std::future::poll_fn(|cx| Poll::Ready(pending.as_mut().poll(cx))).await;
        assert!(matches!(result, Poll::Pending));
    }

    fn resolve_runtime_theme(
        theme: RuntimeTheme,
        appearance: Option<ThemeAppearance>,
        level: ColorLevel,
    ) -> Theme {
        let resolver = theme.into_resolver(ThemeContext {
            appearance,
            color_level: level,
            ..ThemeContext::default()
        });
        *resolver.resolved().palette()
    }

    #[test]
    fn semantic_runtime_themes_follow_terminal_context_without_adapting_explicit_palettes() {
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(ThemeKind::Night),
                Some(ThemeAppearance::Light),
                ColorLevel::Basic,
            ),
            Theme::night().adapted(ColorLevel::Basic)
        );
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(ThemeKind::Day),
                Some(ThemeAppearance::Dark),
                ColorLevel::None,
            ),
            Theme::day().adapted(ColorLevel::None)
        );
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(ThemeKind::Auto),
                Some(ThemeAppearance::Light),
                ColorLevel::Basic,
            ),
            Theme::day().adapted(ColorLevel::Basic)
        );

        let explicit = Theme::night();
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(explicit),
                Some(ThemeAppearance::Light),
                ColorLevel::Basic,
            ),
            explicit
        );
    }

    #[test]
    fn definition_runtime_theme_reresolves_for_late_appearance() {
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(ThemeDefinition::new("runtime-auto", ThemeKind::Auto)),
                Some(ThemeAppearance::Dark),
                ColorLevel::Basic,
            ),
            Theme::night().adapted(ColorLevel::Basic)
        );
        assert_eq!(
            resolve_runtime_theme(
                RuntimeTheme::from(ThemeDefinition::new("runtime-auto", ThemeKind::Auto)),
                Some(ThemeAppearance::Light),
                ColorLevel::Basic,
            ),
            Theme::day().adapted(ColorLevel::Basic)
        );
    }

    #[test]
    fn protocol_lifecycle_is_symmetric_and_releases_kitty_state() {
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
        let mut protocol = TerminalProtocol::default();
        let mut output = Vec::new();

        protocol.activate(&mut output, true, flags).unwrap();
        assert_eq!(pushed_kitty_flags(), flags);

        protocol.restore(&mut output).unwrap();
        assert!(!kitty_flags_pushed());

        let output = String::from_utf8(output).unwrap();
        for (enabled, disabled) in [
            ("\x1b[?1049h", "\x1b[?1049l"),
            ("\x1b[?2004h", "\x1b[?2004l"),
            ("\x1b[?1004h", "\x1b[?1004l"),
            ("\x1b[?1000h", "\x1b[?1000l"),
            ("\x1b[>3u", "\x1b[<1u"),
        ] {
            let enabled_at = output
                .find(enabled)
                .unwrap_or_else(|| panic!("missing activation sequence {enabled:?}"));
            let disabled_at = output
                .find(disabled)
                .unwrap_or_else(|| panic!("missing cleanup sequence {disabled:?}"));
            assert!(
                enabled_at < disabled_at,
                "{enabled:?} must precede its matching cleanup"
            );
        }
    }

    #[derive(Default)]
    struct FailFocus;

    impl Write for FailFocus {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if buf
                .windows(b"?1004h".len())
                .any(|window| window == b"?1004h")
            {
                return Err(io::Error::other("focus setup failed"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn partial_protocol_setup_cleans_only_acquired_features() {
        let mut protocol = TerminalProtocol::default();
        let error = protocol
            .activate(
                &mut FailFocus,
                true,
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
            )
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);

        let mut cleanup = Vec::new();
        protocol.restore(&mut cleanup).unwrap();
        let cleanup = String::from_utf8(cleanup).unwrap();
        assert!(cleanup.contains("\x1b[?2004l"));
        assert!(cleanup.contains("\x1b[?1049l"));
        assert!(!cleanup.contains("\x1b[?1004l"));
        assert!(!cleanup.contains("\x1b[?1000l"));
    }
}
