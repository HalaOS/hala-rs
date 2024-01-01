use std::time::{Duration, Instant};

use hashed_timer::HashedTimeWheel;

use crate::Token;

pub(super) struct MioTimer {
    /// timer start timestamp.
    start_instant: Option<Instant>,
    /// Timeout duration of this timer.
    duration: Duration,
    tick_duration: Option<Duration>,
    /// register expired ticks of [`TimeWheel`](HashedTimeWheel) returns by [`new_timer`](HashedTimeWheel::new_timer)
    timewheel_ticks: Option<u64>,
}

impl MioTimer {
    /// Create new [`MioTimer`] instance with timeout [`duration`](Duration).
    pub(super) fn new(duration: Duration) -> Self {
        Self {
            start_instant: None,
            duration,
            timewheel_ticks: None,
            tick_duration: None,
        }
    }

    /// Return if this timer is started.
    pub(super) fn is_started(&self) -> bool {
        self.start_instant.is_some()
    }

    /// Register start timestamp and add this timer into [`timewheel`](HashedTimeWheel)
    ///
    /// Safety: Only register poller will call this function.
    /// and function [`register_poller`](super::with_poller::MioWithPoller::register_poller) has once call protection.
    ///
    /// # Returns flag
    ///
    /// * Returns true if this timer has been successfully added to the time wheel.
    /// * Returns false if this timer has been timeout.
    pub(super) fn start(
        &mut self,
        token: Token,
        tick_duration: Duration,
        timewheel: &HashedTimeWheel<Token>,
    ) -> bool {
        self.start_instant = Some(Instant::now());
        self.tick_duration = Some(tick_duration);

        self.timewheel_ticks = timewheel.new_timer(token, self.duration);

        self.timewheel_ticks.is_some()
    }

    pub(super) fn is_expired(&self) -> bool {
        if let Some(start_instant) = self.start_instant {
            let elapsed = start_instant.elapsed();

            if elapsed >= self.duration {
                return true;
            }

            if self.duration - elapsed < self.tick_duration.expect("Must call start first") {
                return true;
            }

            return false;
        } else {
            false
        }
    }
}
