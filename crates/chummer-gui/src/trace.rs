//! Opt-in frame timing (`CHUMMER_TRACE_FRAMES=1`), for finding what makes
//! the window stop answering.
//!
//! A frame slower than [`SLOW`] is logged to stderr with the spans that
//! ran in it (`span("recompute")` around a phase), the time spent outside
//! `update` (tessellation and painting) and any wait for the online locks
//! (`chummer_sync::lockwatch`). Spans on other threads are logged on their
//! own when slow, and so are long holds of the online locks by network
//! tasks. Without the variable a span costs one atomic load.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Frames (and background spans) slower than this are logged.
pub const SLOW: Duration = Duration::from_millis(50);

static ENABLED: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

#[derive(Default)]
struct Frame {
    /// Inside `update` on this (the UI) thread.
    open: bool,
    start: Option<Instant>,
    depth: usize,
    /// (start, depth, name, time), in the order the spans ended.
    spans: Vec<(Instant, usize, &'static str, Duration)>,
    /// (lock, wait).
    waits: Vec<(&'static str, Duration)>,
    /// `update` time since eframe last reported a frame's time (egui may
    /// run `update` twice in a frame), to tell painting apart.
    updates: Duration,
    /// The last frame time eframe reported.
    last_cpu: Option<f32>,
    worst: Duration,
    frames: u64,
}

thread_local! {
    static FRAME: RefCell<Frame> = RefCell::new(Frame::default());
}

/// Reads `CHUMMER_TRACE_FRAMES` (call once at start).
pub fn init() {
    let on = std::env::var("CHUMMER_TRACE_FRAMES").is_ok_and(|v| !v.is_empty() && v != "0");
    ENABLED.store(on, Ordering::Relaxed);
    if on {
        START.get_or_init(Instant::now);
        chummer_sync::lockwatch::set_hook(lock_event);
        eprintln!("[trace] frame timing on: frames over {} ms are logged", SLOW.as_millis());
    }
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

fn stamp() -> f64 {
    START.get().map_or(0.0, |s| s.elapsed().as_secs_f64())
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

/// A timed phase; ends when dropped.
#[must_use = "the span ends when this is dropped"]
pub struct Span {
    name: &'static str,
    start: Instant,
    depth: usize,
}

/// Times a phase of the frame (or of a background job) until the returned
/// guard drops. `None` (free) when tracing is off.
pub fn span(name: &'static str) -> Option<Span> {
    if !enabled() {
        return None;
    }
    let depth = FRAME.with(|f| {
        let mut f = f.borrow_mut();
        f.depth += 1;
        f.depth - 1
    });
    Some(Span { name, start: Instant::now(), depth })
}

/// Times `f` as a span.
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let _s = span(name);
    f()
}

impl Drop for Span {
    fn drop(&mut self) {
        let took = self.start.elapsed();
        let open = FRAME.with(|f| {
            let mut f = f.borrow_mut();
            f.depth = f.depth.saturating_sub(1);
            if f.open {
                f.spans.push((self.start, self.depth, self.name, took));
            }
            f.open
        });
        if !open && took >= SLOW {
            let thread = std::thread::current();
            eprintln!("[trace {:>9.3}] {} took {} ms (thread {})", stamp(), self.name, ms(took), thread.name().unwrap_or("?"));
        }
    }
}

fn lock_event(e: chummer_sync::lockwatch::LockEvent) {
    use chummer_sync::lockwatch::LockEvent;
    let in_frame = FRAME.with(|f| {
        let mut f = f.borrow_mut();
        if f.open {
            if let LockEvent::Waited { lock, wait } = e {
                f.waits.push((lock, wait));
            }
        }
        f.open
    });
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("?");
    match e {
        LockEvent::Waited { lock, wait } if !in_frame && wait >= Duration::from_millis(5) => eprintln!("[trace {:>9.3}] waited {} ms for the {lock} lock (thread {name})", stamp(), ms(wait)),
        LockEvent::Held { lock, held } if !in_frame || held >= SLOW => eprintln!("[trace {:>9.3}] held the {lock} lock {} ms (thread {name})", stamp(), ms(held)),
        _ => {}
    }
}

/// Start of `App::update`. `cpu` is eframe's time for the previous frame
/// (`update`, tessellation and painting).
pub fn begin_frame(cpu: Option<f32>) {
    if !enabled() {
        return;
    }
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        if cpu.is_some() && cpu != f.last_cpu {
            // A new report: the frame before this one is done.
            f.last_cpu = cpu;
            let cpu = Duration::from_secs_f32(cpu.unwrap_or_default().max(0.0));
            let paint = cpu.saturating_sub(f.updates);
            f.updates = Duration::ZERO;
            if paint >= SLOW {
                eprintln!("[trace {:>9.3}] previous frame spent {} ms outside update (tessellate/paint/texture upload), {} ms total", stamp(), ms(paint), ms(cpu));
            }
        }
        f.open = true;
        f.start = Some(Instant::now());
        f.depth = 0;
        f.spans.clear();
        f.waits.clear();
    });
}

/// End of `App::update`: logs the frame when it was slow.
pub fn end_frame() {
    if !enabled() {
        return;
    }
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        f.open = false;
        let Some(start) = f.start.take() else { return };
        let took = start.elapsed();
        f.updates += took;
        f.frames += 1;
        if took > f.worst {
            f.worst = took;
        }
        if took < SLOW {
            return;
        }
        let mut line = format!("[trace {:>9.3}] slow frame #{}: update {} ms (worst so far {} ms)", stamp(), f.frames, ms(took), ms(f.worst));
        // Spans of one name at one depth add up (a span per row); shown
        // in the order they first started, nested spans indented.
        let mut spans: Vec<(Instant, usize, &'static str, Duration, usize)> = Vec::new();
        let mut sorted = f.spans.clone();
        sorted.sort_by_key(|(at, depth, _, _)| (*at, *depth));
        for (at, depth, name, d) in sorted {
            match spans.iter_mut().find(|s| s.1 == depth && s.2 == name) {
                Some(s) => {
                    s.3 += d;
                    s.4 += 1;
                }
                None => spans.push((at, depth, name, d, 1)),
            }
        }
        for (_, depth, name, d, n) in spans.iter().filter(|s| s.3 >= Duration::from_millis(2)).take(32) {
            let times = if *n > 1 { format!(" ({n} times)") } else { String::new() };
            line.push_str(&format!("\n    {}{name}: {} ms{times}", "  ".repeat(*depth), ms(*d)));
        }
        for (lock, wait) in &f.waits {
            line.push_str(&format!("\n    waited for the {lock} lock: {} ms", ms(*wait)));
        }
        eprintln!("{line}");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_are_free_when_off() {
        assert!(!enabled() || std::env::var("CHUMMER_TRACE_FRAMES").is_ok());
        if !enabled() {
            assert!(span("x").is_none());
        }
        assert_eq!(time("x", || 3), 3);
    }
}
