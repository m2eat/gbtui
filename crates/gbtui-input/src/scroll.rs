//! Scroll-stream normalization for wheel and trackpad input.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/mouse.rs (STRIP)`
//!
//! Config env prefix is `GBTUI_SCROLL_*`, not `GROK_SCROLL_*`. Flight-recorder
//! HUD is not in v1.

use std::time::{Duration, Instant};

use crossterm::event::{MouseEvent, MouseEventKind};
use gbtui_terminal::{MultiplexerKind, TerminalName};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollInputMode {
    #[default]
    Auto,
    Wheel,
    Trackpad,
}

const STREAM_GAP: Duration = Duration::from_millis(80);
const REDRAW_CADENCE: Duration = Duration::from_millis(16);
const DEFAULT_EVENTS_PER_TICK: u16 = 3;
const DEFAULT_WHEEL_LINES_PER_TICK: u16 = 3;
const DEFAULT_TRACKPAD_LINES_PER_TICK: u16 = 3;
const DEFAULT_WHEEL_TICK_DETECT_MAX: Duration = Duration::from_millis(12);
const DEFAULT_TRACKPAD_DETECT_MAX_INTERVAL: Duration = Duration::from_millis(30);
const ACCEL_MIN_INTERVAL: Duration = Duration::from_millis(6);
const ACCEL_FAST_INTERVAL: Duration = Duration::from_millis(8);
const ACCEL_MEDIUM_INTERVAL: Duration = Duration::from_millis(20);
const ACCEL_HISTORY_SIZE: usize = 6;
const ACCEL_MEDIUM_MULTIPLIER: f32 = 1.6;
const ACCEL_FAST_MULTIPLIER: f32 = 2.5;
const MIN_DELTA_PER_FLUSH: i32 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
}

impl ScrollDirection {
    pub fn from_mouse_event(event: &MouseEvent) -> Option<Self> {
        match event.kind {
            MouseEventKind::ScrollUp => Some(Self::Up),
            MouseEventKind::ScrollDown => Some(Self::Down),
            _ => None,
        }
    }

    fn sign(self) -> i32 {
        match self {
            Self::Up => -1,
            Self::Down => 1,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ScrollConfig {
    pub events_per_tick: u16,
    pub wheel_lines_per_tick: u16,
    pub trackpad_lines_per_tick: u16,
    pub mode: ScrollInputMode,
    pub invert_direction: bool,
    /// Final-line-unit multiplier for both wheel and trackpad input.
    pub speed_multiplier: f32,
    /// Scroll target height, used to bound one cadence flush. Zero keeps the
    /// conservative fallback cap for callers that do not know the viewport.
    pub viewport_height: u16,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            events_per_tick: DEFAULT_EVENTS_PER_TICK,
            wheel_lines_per_tick: DEFAULT_WHEEL_LINES_PER_TICK,
            trackpad_lines_per_tick: DEFAULT_TRACKPAD_LINES_PER_TICK,
            mode: ScrollInputMode::Auto,
            invert_direction: false,
            speed_multiplier: 1.0,
            viewport_height: 0,
        }
    }
}

impl ScrollConfig {
    pub fn from_terminal_context(brand: TerminalName, multiplexer: MultiplexerKind) -> Self {
        let remuxed = matches!(
            multiplexer,
            MultiplexerKind::Tmux
                | MultiplexerKind::Screen
                | MultiplexerKind::Zellij
                | MultiplexerKind::Herdr
        );
        let events_per_tick = if remuxed {
            1
        } else {
            match brand {
                TerminalName::WezTerm
                | TerminalName::Iterm2
                | TerminalName::VsCode
                | TerminalName::Cursor
                | TerminalName::Windsurf
                | TerminalName::Zed => 1,
                _ => DEFAULT_EVENTS_PER_TICK,
            }
        };
        let wheel_lines_per_tick = if remuxed {
            1
        } else {
            match brand {
                TerminalName::Iterm2 | TerminalName::WezTerm => 1,
                _ => DEFAULT_WHEEL_LINES_PER_TICK,
            }
        };
        Self {
            events_per_tick,
            wheel_lines_per_tick,
            trackpad_lines_per_tick: DEFAULT_TRACKPAD_LINES_PER_TICK,
            mode: ScrollInputMode::Auto,
            invert_direction: false,
            speed_multiplier: 1.0,
            viewport_height: 0,
        }
    }

    /// Stamp the current scroll target height onto this event's config.
    pub fn with_viewport_height(mut self, viewport_height: u16) -> Self {
        self.viewport_height = viewport_height;
        self
    }

    /// Maximum line delta that can be emitted by one cadence flush.
    pub fn flush_cap(self) -> i32 {
        (i32::from(self.viewport_height) / 2).max(MIN_DELTA_PER_FLUSH)
    }

    fn apply_direction(self, direction: ScrollDirection) -> ScrollDirection {
        if !self.invert_direction {
            return direction;
        }
        match direction {
            ScrollDirection::Up => ScrollDirection::Down,
            ScrollDirection::Down => ScrollDirection::Up,
        }
    }

    fn speed(self) -> f32 {
        if self.speed_multiplier.is_finite() {
            self.speed_multiplier.max(0.0)
        } else {
            1.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamKind {
    Unknown,
    Wheel,
    Trackpad,
}

#[derive(Debug)]
struct Stream {
    direction: ScrollDirection,
    kind: StreamKind,
    event_count: u32,
    last: Instant,
    start: Instant,
    applied: i32,
    config: ScrollConfig,
    interval_ms: [f32; ACCEL_HISTORY_SIZE],
    interval_len: usize,
    next_interval: usize,
    interval_sum: f32,
    weighted_events: f32,
    events_at_flush: u32,
    coast_spent: i32,
    just_promoted: bool,
}

impl Stream {
    fn new(now: Instant, direction: ScrollDirection, config: ScrollConfig) -> Self {
        let kind = match config.mode {
            ScrollInputMode::Wheel => StreamKind::Wheel,
            ScrollInputMode::Trackpad => StreamKind::Trackpad,
            ScrollInputMode::Auto => StreamKind::Unknown,
        };
        Self {
            direction,
            kind,
            event_count: 0,
            last: now,
            start: now,
            applied: 0,
            config,
            interval_ms: [0.0; ACCEL_HISTORY_SIZE],
            interval_len: 0,
            next_interval: 0,
            interval_sum: 0.0,
            weighted_events: 0.0,
            events_at_flush: 0,
            coast_spent: 0,
            just_promoted: false,
        }
    }

    fn push_event(&mut self, now: Instant) {
        let interval = now.saturating_duration_since(self.last);
        if self.event_count != 0 && interval >= ACCEL_MIN_INTERVAL {
            self.push_interval(interval.as_secs_f32() * 1_000.0);
        }
        self.last = now;
        self.event_count = self.event_count.saturating_add(1);
        self.weighted_events += self.interval_accel();
        if self.is_confirmed_trackpad() {
            self.clamp_trackpad_demand();
        }
    }

    fn push_interval(&mut self, interval_ms: f32) {
        if self.interval_len < ACCEL_HISTORY_SIZE {
            self.interval_ms[self.interval_len] = interval_ms;
            self.interval_len += 1;
        } else {
            self.interval_sum -= self.interval_ms[self.next_interval];
            self.interval_ms[self.next_interval] = interval_ms;
            self.next_interval = (self.next_interval + 1) % ACCEL_HISTORY_SIZE;
        }
        self.interval_sum += interval_ms;
    }

    fn average_interval(&self) -> Option<f32> {
        (self.interval_len != 0).then_some(self.interval_sum / self.interval_len as f32)
    }

    fn interval_accel(&self) -> f32 {
        let Some(average) = self.average_interval() else {
            return 1.0;
        };
        let fast = ACCEL_FAST_INTERVAL.as_secs_f32() * 1_000.0;
        let medium = ACCEL_MEDIUM_INTERVAL.as_secs_f32() * 1_000.0;
        if average <= fast {
            ACCEL_FAST_MULTIPLIER
        } else if average <= medium {
            let progress = (average - fast) / (medium - fast);
            ACCEL_FAST_MULTIPLIER + progress * (ACCEL_MEDIUM_MULTIPLIER - ACCEL_FAST_MULTIPLIER)
        } else {
            1.0
        }
    }

    fn maybe_promote(&mut self, now: Instant) {
        if self.config.mode != ScrollInputMode::Auto || self.kind != StreamKind::Unknown {
            return;
        }
        let events_per_tick = self.config.events_per_tick.max(1) as u32;
        if events_per_tick == 1
            && self.event_count > 2
            && self.average_interval().is_some_and(|average| {
                average < DEFAULT_TRACKPAD_DETECT_MAX_INTERVAL.as_secs_f32() * 1_000.0
            })
        {
            self.kind = StreamKind::Trackpad;
            self.clamp_trackpad_demand();
        } else if events_per_tick > 1
            && self.event_count >= events_per_tick
            && now.saturating_duration_since(self.start) <= DEFAULT_WHEEL_TICK_DETECT_MAX
        {
            self.kind = StreamKind::Wheel;
            self.just_promoted = true;
        }
    }

    fn finalize_kind(&mut self) {
        self.kind = match self.config.mode {
            ScrollInputMode::Wheel => StreamKind::Wheel,
            ScrollInputMode::Trackpad => StreamKind::Trackpad,
            ScrollInputMode::Auto if self.kind != StreamKind::Unknown => self.kind,
            ScrollInputMode::Auto if self.config.events_per_tick <= 1 && self.event_count <= 2 => {
                StreamKind::Wheel
            }
            ScrollInputMode::Auto => StreamKind::Trackpad,
        };
    }

    fn is_confirmed_trackpad(&self) -> bool {
        matches!(self.config.mode, ScrollInputMode::Trackpad)
            || (self.config.mode == ScrollInputMode::Auto && self.kind == StreamKind::Trackpad)
    }

    fn is_wheel_like(&self) -> bool {
        matches!(self.config.mode, ScrollInputMode::Wheel)
            || self.kind == StreamKind::Wheel
            || (self.config.mode == ScrollInputMode::Auto
                && self.kind == StreamKind::Unknown
                && self.config.events_per_tick <= 1)
    }

    fn unaccelerated_line_rate(&self) -> f32 {
        let lines_per_tick = if self.is_wheel_like() {
            self.config.wheel_lines_per_tick
        } else {
            self.config.trackpad_lines_per_tick
        };
        lines_per_tick.max(1) as f32 / self.config.events_per_tick.max(1) as f32
            * self.config.speed()
    }

    fn trackpad_line_rate(&self) -> f32 {
        self.config.trackpad_lines_per_tick.max(1) as f32 / DEFAULT_EVENTS_PER_TICK as f32
            * self.config.speed()
    }

    fn desired_magnitude(&self, carry: f32) -> f32 {
        if self.is_confirmed_trackpad() {
            self.weighted_events * self.trackpad_line_rate() + carry
        } else {
            self.event_count as f32 * self.unaccelerated_line_rate()
        }
    }

    fn clamp_trackpad_demand(&mut self) {
        let rate = self.trackpad_line_rate();
        if rate <= f32::EPSILON {
            return;
        }
        let raw = self.event_count as f32 * rate;
        let honorable = self.applied.abs().saturating_add(self.config.flush_cap()) as f32;
        let ceiling = raw.max(honorable);
        if self.weighted_events * rate > ceiling {
            self.weighted_events = ceiling / rate;
        }
    }

    fn limit_finalize_reprice(&mut self, desired_before: f32, carry: f32) {
        if !self.is_confirmed_trackpad() {
            return;
        }
        let rate = self.trackpad_line_rate();
        if rate <= f32::EPSILON || self.desired_magnitude(carry) <= desired_before {
            return;
        }
        self.weighted_events = ((desired_before - carry).max(0.0)) / rate;
    }

    fn effective_pending(&self, carry: f32) -> i32 {
        let mut desired = self.desired_magnitude(carry).trunc() as i32;
        if self.is_wheel_like() && desired == 0 && self.event_count != 0 {
            desired = 1;
        }
        let signed_desired = desired.saturating_mul(self.direction.sign());
        let pending = signed_desired.saturating_sub(self.applied);
        match self.direction {
            ScrollDirection::Up => pending.min(0),
            ScrollDirection::Down => pending.max(0),
        }
    }

    fn coasting(&self) -> bool {
        self.event_count == self.events_at_flush
    }

    fn flushable(&self, carry: f32) -> i32 {
        let pending = self.effective_pending(carry);
        let cap = self.config.flush_cap();
        let magnitude = if self.coasting() {
            let line_floor = if self.is_confirmed_trackpad() {
                self.config.trackpad_lines_per_tick.max(1) as i32
            } else {
                self.config.wheel_lines_per_tick.max(1) as i32
            };
            let taper = (pending.abs() / 2).max(line_floor);
            pending
                .abs()
                .min(taper)
                .min((cap - self.coast_spent).max(0))
        } else {
            pending.abs().min(cap)
        };
        pending.signum() * magnitude
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollUpdate {
    pub lines: i32,
    pub next_tick_in: Option<Duration>,
}

#[derive(Debug)]
pub struct MouseScrollState {
    stream: Option<Stream>,
    last_redraw_at: Instant,
    redraw_cadence: Duration,
    carry_lines: f32,
    carry_direction: Option<ScrollDirection>,
}

impl MouseScrollState {
    /// Create a state with a caller-supplied clock origin.
    pub fn new(now: Instant) -> Self {
        Self {
            stream: None,
            last_redraw_at: now,
            redraw_cadence: REDRAW_CADENCE,
            carry_lines: 0.0,
            carry_direction: None,
        }
    }

    pub fn on_scroll_event_at(
        &mut self,
        now: Instant,
        direction: ScrollDirection,
        config: ScrollConfig,
    ) -> ScrollUpdate {
        let direction = config.apply_direction(direction);
        let mut lines = 0;

        if let Some(mut stream) = self.stream.take() {
            let gap = now.saturating_duration_since(stream.last);
            let reversed = stream.direction != direction;
            if gap >= STREAM_GAP || reversed {
                lines += self.finalize_stream(now, &mut stream, reversed);
            } else {
                self.stream = Some(stream);
            }
        }

        if self.stream.is_none() {
            if self.carry_direction != Some(direction) {
                self.carry_lines = 0.0;
                self.carry_direction = Some(direction);
            }
            self.stream = Some(Stream::new(now, direction, config));
        }

        let carry = self.carry_lines;
        let stream = self.stream.as_mut().expect("stream inserted above");
        stream.push_event(now);
        stream.maybe_promote(now);
        if now.saturating_duration_since(self.last_redraw_at) >= self.redraw_cadence
            || stream.kind == StreamKind::Wheel
            || stream.just_promoted
        {
            lines += Self::flush_stream(&mut self.last_redraw_at, carry, now, stream);
            stream.just_promoted = false;
        }

        ScrollUpdate {
            lines,
            next_tick_in: self.next_tick_in(now),
        }
    }

    pub fn on_tick_at(&mut self, now: Instant) -> ScrollUpdate {
        let mut lines = 0;
        if let Some(mut stream) = self.stream.take() {
            let gap = now.saturating_duration_since(stream.last);
            let flushable = stream.flushable(self.carry_lines);
            if gap >= STREAM_GAP && flushable == 0 {
                lines = self.finalize_stream(now, &mut stream, false);
            } else {
                if flushable != 0
                    && now.saturating_duration_since(self.last_redraw_at) >= self.redraw_cadence
                {
                    lines = Self::flush_stream(
                        &mut self.last_redraw_at,
                        self.carry_lines,
                        now,
                        &mut stream,
                    );
                }
                if gap >= STREAM_GAP && stream.flushable(self.carry_lines) == 0 {
                    lines += self.finalize_stream(now, &mut stream, false);
                } else {
                    self.stream = Some(stream);
                }
            }
        }
        ScrollUpdate {
            lines,
            next_tick_in: self.next_tick_in(now),
        }
    }

    pub fn has_active_stream(&self) -> bool {
        self.stream.is_some()
    }

    /// Cancel pending motion, including fractional carry, when its target is
    /// no longer safe to receive pointer input.
    pub fn cancel_stream(&mut self) {
        self.stream = None;
        self.carry_lines = 0.0;
        self.carry_direction = None;
    }

    /// Delay until the dedicated scroll clock must run, if a stream is live.
    /// This is independent from application animation demand.
    pub fn scroll_clock_deadline(&self, now: Instant) -> Option<Duration> {
        self.stream.as_ref()?;
        Some(self.next_tick_in(now).unwrap_or(Duration::ZERO))
    }

    fn flush_stream(
        last_redraw_at: &mut Instant,
        carry: f32,
        now: Instant,
        stream: &mut Stream,
    ) -> i32 {
        let delta = stream.flushable(carry);
        if delta == 0 {
            return 0;
        }
        if stream.coasting() {
            stream.coast_spent = stream.coast_spent.saturating_add(delta.abs());
        }
        stream.applied = stream.applied.saturating_add(delta);
        stream.events_at_flush = stream.event_count;
        *last_redraw_at = now;
        delta
    }

    fn finalize_stream(&mut self, now: Instant, stream: &mut Stream, cancel_backlog: bool) -> i32 {
        let carry = self.carry_lines;
        let desired_before = stream.desired_magnitude(carry);
        stream.finalize_kind();
        stream.limit_finalize_reprice(desired_before, carry);
        let lines = if cancel_backlog {
            0
        } else {
            Self::flush_stream(&mut self.last_redraw_at, carry, now, stream)
        };
        self.carry_lines = if cancel_backlog || !stream.is_confirmed_trackpad() {
            0.0
        } else {
            (stream.desired_magnitude(carry) - stream.applied.abs() as f32).fract()
        };
        lines
    }

    fn next_tick_in(&self, now: Instant) -> Option<Duration> {
        let stream = self.stream.as_ref()?;
        let gap = now.saturating_duration_since(stream.last);
        let until_cadence = self
            .redraw_cadence
            .saturating_sub(now.saturating_duration_since(self.last_redraw_at));
        let flushable = stream.flushable(self.carry_lines) != 0;

        if gap >= STREAM_GAP {
            return flushable.then_some(until_cadence);
        }

        let until_gap = STREAM_GAP.saturating_sub(gap);
        Some(if flushable {
            until_gap.min(until_cadence)
        } else {
            until_gap
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> Instant {
        Instant::now()
    }

    fn drain(state: &mut MouseScrollState, mut now: Instant) -> i32 {
        let mut lines = 0;
        for _ in 0..32 {
            let Some(delay) = state.scroll_clock_deadline(now) else {
                return lines;
            };
            now += delay.max(Duration::from_millis(1));
            lines += state.on_tick_at(now).lines;
        }
        panic!("scroll stream did not become idle");
    }

    fn trackpad_config() -> ScrollConfig {
        ScrollConfig {
            mode: ScrollInputMode::Trackpad,
            ..ScrollConfig::default()
        }
    }

    #[test]
    fn single_trackpad_step_flushes_on_its_deadline() {
        let base = t0();
        let mut state = MouseScrollState::new(base);
        let update = state.on_scroll_event_at(
            base + Duration::from_millis(1),
            ScrollDirection::Down,
            trackpad_config(),
        );
        assert_eq!(update.lines, 0);

        let deadline = state
            .scroll_clock_deadline(base + Duration::from_millis(1))
            .expect("single step arms the scroll clock");
        let flushed = state.on_tick_at(base + Duration::from_millis(1) + deadline);
        assert_eq!(flushed.lines, 1);
        let _ = drain(&mut state, base + Duration::from_millis(1) + deadline);
        assert!(!state.has_active_stream());
    }

    #[test]
    fn speed_multiplier_scales_delivered_line_units() {
        let base = t0();
        let config = ScrollConfig {
            mode: ScrollInputMode::Trackpad,
            speed_multiplier: 2.0,
            ..ScrollConfig::default()
        };
        let mut state = MouseScrollState::new(base);
        let update = state.on_scroll_event_at(
            base + Duration::from_millis(16),
            ScrollDirection::Down,
            config,
        );
        assert_eq!(update.lines, 2);
    }

    #[test]
    fn real_fast_intervals_accelerate_but_dense_batches_do_not() {
        fn gesture(interval: u64) -> i32 {
            let base = t0();
            let mut state = MouseScrollState::new(base);
            let mut total = 0;
            let mut last = base;
            for event in 0..6 {
                last = base + Duration::from_millis(1 + event * interval);
                total += state
                    .on_scroll_event_at(last, ScrollDirection::Down, trackpad_config())
                    .lines;
            }
            total + drain(&mut state, last)
        }

        let slow = gesture(40);
        let fast = gesture(7);
        let batched = gesture(2);
        assert!(
            fast > slow,
            "real fast intervals should accelerate: {fast} <= {slow}"
        );
        assert_eq!(
            batched, slow,
            "sub-human report batches must not enter the acceleration bands"
        );
    }

    #[test]
    fn direction_reversal_discards_pending_old_direction() {
        let base = t0();
        let mut state = MouseScrollState::new(base);
        let config = trackpad_config();
        let _ = state.on_scroll_event_at(
            base + Duration::from_millis(1),
            ScrollDirection::Down,
            config,
        );
        let reversed =
            state.on_scroll_event_at(base + Duration::from_millis(2), ScrollDirection::Up, config);
        let mut total = reversed.lines;
        total += state.on_tick_at(base + Duration::from_millis(16)).lines;
        total += drain(&mut state, base + Duration::from_millis(16));
        assert_eq!(total, -1);
    }

    #[test]
    fn fractional_trackpad_remainder_carries_between_gestures() {
        let base = t0();
        let config = ScrollConfig {
            mode: ScrollInputMode::Trackpad,
            trackpad_lines_per_tick: 1,
            ..ScrollConfig::default()
        };
        let mut state = MouseScrollState::new(base);
        let mut total = 0;
        for offset in [1, 200, 400] {
            let at = base + Duration::from_millis(offset);
            total += state
                .on_scroll_event_at(at, ScrollDirection::Down, config)
                .lines;
            total += drain(&mut state, at);
        }
        assert_eq!(total, 1);
    }

    #[test]
    fn capped_tail_coasts_to_idle_without_spinning() {
        let base = t0();
        let mut state = MouseScrollState::new(base);
        let config = trackpad_config().with_viewport_height(20);
        let last = base + Duration::from_millis(1);
        for _ in 0..20 {
            let update = state.on_scroll_event_at(last, ScrollDirection::Down, config);
            assert!(update.lines.abs() <= config.flush_cap());
        }
        let mut now = last;
        let mut delivered = 0;
        for _ in 0..32 {
            let Some(delay) = state.scroll_clock_deadline(now) else {
                break;
            };
            now += delay.max(Duration::from_millis(1));
            let update = state.on_tick_at(now);
            assert!(update.lines.abs() <= config.flush_cap());
            delivered += update.lines;
        }
        assert!(delivered > 0);
        assert!(!state.has_active_stream());
        assert_eq!(state.scroll_clock_deadline(now), None);
    }

    #[test]
    fn wheel_mode_three_events_make_three_lines() {
        let base = t0();
        let mut state = MouseScrollState::new(base);
        let config = ScrollConfig {
            mode: ScrollInputMode::Wheel,
            events_per_tick: 3,
            wheel_lines_per_tick: 3,
            ..ScrollConfig::default()
        };
        let mut total = 0;
        for event in 0..3 {
            total += state
                .on_scroll_event_at(
                    base + Duration::from_millis(event),
                    ScrollDirection::Down,
                    config,
                )
                .lines;
        }
        assert_eq!(total, 3);
    }

    #[test]
    fn terminal_fallback_keeps_one_event_wheel_notches() {
        let base = t0();
        let config =
            ScrollConfig::from_terminal_context(TerminalName::Ghostty, MultiplexerKind::Tmux);
        let mut state = MouseScrollState::new(base);
        let first = state
            .on_scroll_event_at(base, ScrollDirection::Down, config)
            .lines;
        let total = first + drain(&mut state, base);
        assert_eq!(total, 1);
    }

    #[test]
    fn long_gap_finalizes_before_the_next_gesture() {
        let base = t0();
        let config = trackpad_config();
        let mut state = MouseScrollState::new(base);
        let first = state.on_scroll_event_at(
            base + Duration::from_millis(1),
            ScrollDirection::Down,
            config,
        );
        let second = state.on_scroll_event_at(
            base + Duration::from_millis(200),
            ScrollDirection::Down,
            config,
        );
        let total =
            first.lines + second.lines + drain(&mut state, base + Duration::from_millis(200));
        assert_eq!(total, 2);
    }
}
