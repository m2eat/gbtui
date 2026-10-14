//! Demand-driven animation ticks.
//!
//! `SOURCE: crates/codegen/xai-grok-pager/src/app/app_view.rs TickDemand (REWRITE)`
//! plus `wave_brightness` / `pulse_brightness` from
//! `xai-grok-pager-render/src/theme/tokyonight.rs (COPY)`.

use std::time::{Duration, Instant};

/// How urgently the UI needs another animation frame.
///
/// Idle UIs must stay at [`TickDemand::None`] so the event loop does not spin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TickDemand {
    #[default]
    None,
    /// ~12 fps: slow shimmer, modifier-hover poll.
    Slow,
    /// Default animation fps (30).
    Fast,
}

impl TickDemand {
    pub const SLOW_INTERVAL: Duration = Duration::from_millis(83);
    pub const DEFAULT_FAST_FPS: u8 = 30;

    pub fn interval(self, fast_fps: u8) -> Option<Duration> {
        match self {
            Self::None => None,
            Self::Slow => Some(Self::SLOW_INTERVAL),
            Self::Fast => {
                let fps = fast_fps.max(1) as u64;
                Some(Duration::from_millis(1000 / fps))
            }
        }
    }
}

/// Wall-clock animation state for one frame.
#[derive(Debug, Clone, Copy)]
pub struct AnimationClock {
    epoch: Instant,
    now: Instant,
    frame: u64,
    demand: TickDemand,
    fast_fps: u8,
    wave_rows: u16,
}

impl AnimationClock {
    pub const DEFAULT_WAVE_ROWS: u16 = 32;

    pub fn new(now: Instant) -> Self {
        Self {
            epoch: now,
            now,
            frame: 0,
            demand: TickDemand::None,
            fast_fps: TickDemand::DEFAULT_FAST_FPS,
            wave_rows: Self::DEFAULT_WAVE_ROWS,
        }
    }

    pub fn advance(&mut self, now: Instant) {
        self.now = now;
        self.frame = self.frame.wrapping_add(1);
        self.demand = TickDemand::None;
    }

    pub fn now(&self) -> Instant {
        self.now
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn request(&mut self, demand: TickDemand) {
        if demand > self.demand {
            self.demand = demand;
        }
    }

    /// Drop last frame's demand so `paint` only keeps what this draw requests.
    pub fn clear_demand(&mut self) {
        self.demand = TickDemand::None;
    }

    pub fn demand(&self) -> TickDemand {
        self.demand
    }

    /// Phase in `[0, 1)` for a repeating period.
    pub fn phase(&self, period: Duration) -> f32 {
        let period = period.max(Duration::from_millis(1)).as_secs_f32();
        (self.now.duration_since(self.epoch).as_secs_f32() / period).fract()
    }

    pub fn wave_brightness(&self, row: u16) -> f32 {
        wave_brightness(self.frame, row, self.wave_rows, 0.15)
    }

    pub fn pulse_brightness(&self) -> f32 {
        pulse_brightness(self.frame, 0.08)
    }

    pub fn set_fast_fps(&mut self, fps: u8) {
        self.fast_fps = fps.max(1);
    }

    pub fn fast_fps(&self) -> u8 {
        self.fast_fps
    }
}

/// Traveling wave along a column of rows. Returns brightness in `[0, 1]`.
pub fn wave_brightness(tick: u64, row: u16, wave_rows: u16, speed: f32) -> f32 {
    use std::f32::consts::PI;
    let rows_per_wave = wave_rows.max(1) as f32;
    let phase = (row as f32 / rows_per_wave) * 2.0 * PI;
    let t = tick as f32 * speed;
    let sin_val = (t + phase).sin();
    sin_val * sin_val
}

/// Temporal pulse shared by every element on the same tick.
pub fn pulse_brightness(tick: u64, speed: f32) -> f32 {
    let t = tick as f32 * speed;
    let sin_val = t.sin();
    sin_val * sin_val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demand_orders_none_slow_fast() {
        assert!(TickDemand::None < TickDemand::Slow);
        assert!(TickDemand::Slow < TickDemand::Fast);
    }

    #[test]
    fn none_has_no_interval() {
        assert_eq!(TickDemand::None.interval(30), None);
    }

    #[test]
    fn wave_stays_in_unit_interval() {
        for tick in 0..64 {
            for row in 0..32 {
                let b = wave_brightness(tick, row, 32, 0.15);
                assert!((0.0..=1.0).contains(&b));
            }
        }
    }

    #[test]
    fn request_keeps_the_highest_demand() {
        let t0 = Instant::now();
        let mut clock = AnimationClock::new(t0);
        clock.request(TickDemand::Slow);
        clock.request(TickDemand::None);
        assert_eq!(clock.demand(), TickDemand::Slow);
        clock.request(TickDemand::Fast);
        assert_eq!(clock.demand(), TickDemand::Fast);
    }
}
