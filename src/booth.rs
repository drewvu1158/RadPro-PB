//! Each picture is requested by the guest: preview/ready -> countdown -> shot.
//!
//! This module has no I/O. The app feeds it elapsed milliseconds and calls
//! [`Booth::deliver_shot`] once the captured image arrives. Between pictures,
//! the clock is idle until the guest requests another shot. That keeps the
//! whole flow testable without a camera or a GUI.

/// How the session is going right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Live camera preview, waiting for the operator to start a session.
    Preview,
    /// Waiting for the guest to request another picture in the current session.
    Ready,
    /// Counting down to the next shot.
    Countdown,
    /// Countdown hit zero; waiting for the app to hand over the captured frame.
    Shooting,
    /// All shots taken and composed; the strip is ready to view/save.
    Review,
}

/// Side effects the app must perform after [`Booth::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoothEvent {
    /// Nothing to do this tick.
    None,
    /// Take a shot now. `index` is the zero-based shot number.
    ShotDue { index: u32 },
    /// The final shot was delivered; compose and show the strip.
    StripComplete,
}

/// A photo booth session in progress.
#[derive(Clone, Debug)]
pub struct Booth {
    shots_wanted: u32,
    countdown_ms: u64,
    phase: Phase,
    shots_taken: u32,
    elapsed_ms: u64,
}

impl Booth {
    pub fn new(shots_wanted: u32, countdown_seconds: u32) -> Self {
        Self {
            shots_wanted: shots_wanted.max(1),
            countdown_ms: u64::from(countdown_seconds.max(1)) * 1000,
            phase: Phase::Preview,
            shots_taken: 0,
            elapsed_ms: 0,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn shots_taken(&self) -> u32 {
        self.shots_taken
    }

    pub fn shots_wanted(&self) -> u32 {
        self.shots_wanted
    }

    pub fn capture_active(&self) -> bool {
        matches!(
            self.phase,
            Phase::Ready | Phase::Countdown | Phase::Shooting
        )
    }

    pub fn ready_for_picture(&self) -> bool {
        matches!(self.phase, Phase::Preview | Phase::Ready)
    }

    /// Milliseconds left in the current countdown (0 outside Countdown).
    pub fn remaining_ms(&self) -> u64 {
        match self.phase {
            Phase::Countdown => self.countdown_ms.saturating_sub(self.elapsed_ms),
            _ => 0,
        }
    }

    /// Whole seconds shown on the countdown overlay, ceil-style, minimum 1.
    pub fn remaining_seconds(&self) -> u64 {
        self.remaining_ms().div_ceil(1000).max(1)
    }

    /// Start one countdown. Requests during capture or review are ignored.
    pub fn request_shot(&mut self) -> BoothEvent {
        match self.phase {
            Phase::Preview | Phase::Ready => {
                self.phase = Phase::Countdown;
                self.elapsed_ms = 0;
                BoothEvent::None
            }
            _ => BoothEvent::None,
        }
    }

    /// Advance the clock. `dt_ms` is the time since the previous call.
    pub fn update(&mut self, dt_ms: u64) -> BoothEvent {
        match self.phase {
            Phase::Countdown => {
                self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
                if self.elapsed_ms >= self.countdown_ms {
                    self.phase = Phase::Shooting;
                    BoothEvent::ShotDue {
                        index: self.shots_taken,
                    }
                } else {
                    BoothEvent::None
                }
            }
            _ => BoothEvent::None,
        }
    }

    /// Deliver the frame captured for the pending [`BoothEvent::ShotDue`].
    /// Returns `StripComplete` when the last shot is in.
    pub fn deliver_shot(&mut self) -> BoothEvent {
        if self.phase != Phase::Shooting {
            return BoothEvent::None;
        }
        self.shots_taken += 1;
        if self.shots_taken >= self.shots_wanted {
            self.phase = Phase::Review;
            BoothEvent::StripComplete
        } else {
            self.phase = Phase::Ready;
            self.elapsed_ms = 0;
            BoothEvent::None
        }
    }

    /// Keep completed photos and allow the pending shot to be retried.
    pub fn capture_failed(&mut self) {
        if self.phase == Phase::Shooting {
            self.phase = Phase::Ready;
            self.elapsed_ms = 0;
        }
    }

    /// Abandon the session and go back to Preview (clears captured shots).
    pub fn retake(&mut self) {
        self.shots_taken = 0;
        self.elapsed_ms = 0;
        self.phase = Phase::Preview;
    }

    /// After the strip has been shown/saved, reset for the next guest.
    pub fn finish(&mut self) {
        self.retake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(booth: &mut Booth, dt_ms: u64, ticks: usize) -> Vec<BoothEvent> {
        (0..ticks).map(|_| booth.update(dt_ms)).collect()
    }

    #[test]
    fn full_session_strip_flow() {
        let mut booth = Booth::new(4, 3);
        assert_eq!(booth.phase(), Phase::Preview);
        for shot in 0..4 {
            assert_eq!(booth.update(60_000), BoothEvent::None);
            assert_eq!(booth.shots_taken(), shot);
            assert_eq!(booth.request_shot(), BoothEvent::None);
            assert_eq!(booth.phase(), Phase::Countdown);
            // 3 seconds of 100 ms ticks per shot.
            let events = step(&mut booth, 100, 30);
            assert!(events[..29].iter().all(|e| *e == BoothEvent::None));
            assert_eq!(events[29], BoothEvent::ShotDue { index: shot });
            assert_eq!(booth.phase(), Phase::Shooting);

            let after = booth.deliver_shot();
            if shot < 3 {
                assert_eq!(after, BoothEvent::None);
                assert_eq!(booth.phase(), Phase::Ready);
            } else {
                assert_eq!(after, BoothEvent::StripComplete);
                assert_eq!(booth.phase(), Phase::Review);
            }
        }
        assert_eq!(booth.shots_taken(), 4);
        booth.finish();
        assert_eq!(booth.phase(), Phase::Preview);
        assert_eq!(booth.shots_taken(), 0);
    }

    #[test]
    fn countdown_counts_down_in_whole_seconds() {
        let mut booth = Booth::new(1, 3);
        booth.request_shot();
        assert_eq!(booth.remaining_seconds(), 3);
        booth.update(1000);
        assert_eq!(booth.remaining_seconds(), 2);
        booth.update(1000);
        assert_eq!(booth.remaining_seconds(), 1);
        booth.update(999);
        assert_eq!(booth.remaining_seconds(), 1);
        booth.update(1);
        assert_eq!(booth.phase(), Phase::Shooting);
        assert_eq!(booth.remaining_ms(), 0);
    }

    #[test]
    fn retake_from_review_resets() {
        let mut booth = Booth::new(1, 1);
        booth.request_shot();
        step(&mut booth, 1000, 1);
        booth.deliver_shot();
        assert_eq!(booth.phase(), Phase::Review);
        booth.retake();
        assert_eq!(booth.phase(), Phase::Preview);
        assert_eq!(booth.shots_taken(), 0);
    }

    #[test]
    fn deliver_shot_ignored_outside_shooting() {
        let mut booth = Booth::new(1, 1);
        assert_eq!(booth.deliver_shot(), BoothEvent::None);
        booth.request_shot();
        assert_eq!(booth.deliver_shot(), BoothEvent::None);
    }

    #[test]
    fn single_shot_session() {
        let mut booth = Booth::new(1, 1);
        booth.request_shot();
        let events = step(&mut booth, 1000, 1);
        assert_eq!(events[0], BoothEvent::ShotDue { index: 0 });
        assert_eq!(booth.deliver_shot(), BoothEvent::StripComplete);
    }

    #[test]
    fn busy_clicks_do_not_restart_countdown_and_failed_captures_can_be_retried() {
        let mut booth = Booth::new(2, 3);
        booth.request_shot();
        booth.update(1000);
        booth.request_shot();
        assert_eq!(booth.remaining_seconds(), 2);
        assert_eq!(booth.update(2000), BoothEvent::ShotDue { index: 0 });
        booth.request_shot();
        assert_eq!(booth.phase(), Phase::Shooting);
        booth.deliver_shot();
        assert_eq!(booth.phase(), Phase::Ready);
        assert_eq!(booth.update(u64::MAX), BoothEvent::None);
        booth.request_shot();
        assert_eq!(booth.update(3000), BoothEvent::ShotDue { index: 1 });
        booth.capture_failed();
        assert_eq!(booth.shots_taken(), 1);
        assert_eq!(booth.phase(), Phase::Ready);
        assert!(booth.capture_active());
        assert!(booth.ready_for_picture());
        booth.request_shot();
        assert_eq!(booth.update(3000), BoothEvent::ShotDue { index: 1 });
        assert_eq!(booth.deliver_shot(), BoothEvent::StripComplete);
        booth.request_shot();
        assert_eq!(booth.phase(), Phase::Review);
    }
}
