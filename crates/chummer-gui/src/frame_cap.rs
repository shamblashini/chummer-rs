//! Frame pacing without vsync.
//!
//! The app runs with vsync off (see `main`): a vsync'd swap on Wayland
//! blocks while the window is hidden, and the compositor then flags the
//! app as not responding. egui only repaints on demand, but spinners and
//! animations ask for a repaint every frame, so frames are capped here at
//! about 60 per second. The wait is short and bounded, so the event loop
//! keeps answering the compositor. `CHUMMER_VSYNC=1` turns vsync back on.

use std::cell::Cell;
use std::time::{Duration, Instant};

/// Shortest time between two frames.
const MIN_FRAME: Duration = Duration::from_micros(16_000);

thread_local! {
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Whether to ask the driver for vsync.
pub fn vsync() -> bool {
    std::env::var_os("CHUMMER_VSYNC").is_some_and(|v| v != "0")
}

/// How long to wait before the next frame, given the last one's start.
fn remaining(last: Option<Instant>, now: Instant) -> Duration {
    last.map_or(Duration::ZERO, |l| MIN_FRAME.saturating_sub(now.saturating_duration_since(l)))
}

/// Called at the start of every frame: sleeps the rest of the frame
/// budget (at most [`MIN_FRAME`]) when frames come faster than ~60/s.
pub fn wait() {
    if vsync() {
        return;
    }
    LAST.with(|last| {
        let wait = remaining(last.get(), Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        last.set(Some(Instant::now()));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_only_for_the_rest_of_a_frame() {
        let t = Instant::now();
        assert_eq!(remaining(None, t), Duration::ZERO);
        assert_eq!(remaining(Some(t), t + Duration::from_millis(20)), Duration::ZERO);
        let w = remaining(Some(t), t + Duration::from_millis(6));
        assert!(w <= MIN_FRAME && w >= Duration::from_millis(9), "{w:?}");
    }
}
