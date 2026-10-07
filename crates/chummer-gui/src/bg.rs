//! Slow work off the UI thread: saving, loading, printing, file dialogs.
//!
//! The window must answer the compositor every frame (Wayland desktops
//! call an app "not responding" after a few seconds without an answer),
//! so anything that can take longer than a frame runs on its own thread:
//! [`spawn`] starts it under an id, and the code that asked takes the
//! result with [`take`] on a later frame (the thread asks for a repaint
//! when it is done). While anything runs, the status bar shows what
//! ([`running`]).
//!
//! File dialogs go through here too: rfd's dialogs block the thread that
//! opens them until the user picks, and on the UI thread that froze the
//! window for as long as the dialog was open.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;

struct Job {
    /// What the status bar says while it runs.
    label: String,
    started: Instant,
    /// The result, once there.
    done: Arc<Mutex<Option<Box<dyn Any + Send>>>>,
    handle: Option<JoinHandle<()>>,
}

fn jobs() -> &'static Mutex<HashMap<String, Job>> {
    static JOBS: OnceLock<Mutex<HashMap<String, Job>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<String, Job>> {
    jobs().lock().unwrap_or_else(|e| e.into_inner())
}

static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// The app's context, for [`run`] (set once at start).
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// [`spawn`] with the app's context (code that has no `egui::Context`
/// at hand).
pub fn run<T: Send + 'static>(id: impl Into<String>, label: impl Into<String>, f: impl FnOnce() -> T + Send + 'static) -> bool {
    let ctx = CONTEXT.get().cloned().unwrap_or_default();
    spawn(&ctx, id, label, f)
}

/// A job's thread panicked (its result, in place of the value).
struct Panicked(String);

/// Runs `f` on a new thread under `id`. Returns false (and runs nothing)
/// while a job with that id is still running. A finished job whose
/// result nobody took (its window closed meanwhile) is dropped.
pub fn spawn<T: Send + 'static>(ctx: &egui::Context, id: impl Into<String>, label: impl Into<String>, f: impl FnOnce() -> T + Send + 'static) -> bool {
    let id = id.into();
    let mut jobs = lock();
    if let Some(j) = jobs.get(&id) {
        if j.done.lock().map(|d| d.is_none()).unwrap_or(true) {
            return false;
        }
        jobs.remove(&id);
    }
    let done: Arc<Mutex<Option<Box<dyn Any + Send>>>> = Arc::default();
    let slot = done.clone();
    let ctx = ctx.clone();
    let name: String = id.chars().take(15).collect();
    let trace_name = id.clone();
    let handle = std::thread::Builder::new()
        .name(format!("bg {name}"))
        .spawn(move || {
            let start = Instant::now();
            let v: Box<dyn Any + Send> = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                Ok(v) => Box::new(v),
                Err(e) => {
                    let msg = e.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| e.downcast_ref::<String>().cloned()).unwrap_or_default();
                    Box::new(Panicked(msg))
                }
            };
            if crate::trace::enabled() {
                eprintln!("[trace] background job {trace_name} took {:.1} ms", start.elapsed().as_secs_f64() * 1000.0);
            }
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
            ctx.request_repaint();
        })
        .expect("a thread");
    jobs.insert(id, Job { label: label.into(), started: Instant::now(), done, handle: Some(handle) });
    true
}

/// Opens a file dialog (`f` builds and shows it) on its own thread under
/// `id`; take the answer with [`take`]. Returns false while one with that
/// id is open.
pub fn dialog<T: Send + 'static>(ctx: &egui::Context, id: impl Into<String>, f: impl FnOnce() -> T + Send + 'static) -> bool {
    spawn(ctx, id, "Waiting for the file dialog…", f)
}

/// The result of job `id`, once it finished (then the id is free again).
/// `None` while it runs, when there is no such job, or when the result
/// is not a `T`. A job that panicked is dropped (`None`, and [`busy`] is
/// false again: callers waiting for it treat that as a failure).
pub fn take<T: 'static>(id: &str) -> Option<T> {
    let mut jobs = lock();
    let job = jobs.get(id)?;
    let v = job.done.lock().unwrap_or_else(|e| e.into_inner()).take()?;
    let v = match v.downcast::<Panicked>() {
        Ok(p) => {
            eprintln!("chummer-rs: background job {id} failed: {}", p.0);
            jobs.remove(id);
            return None;
        }
        Err(v) => v,
    };
    match v.downcast::<T>() {
        Ok(v) => {
            if let Some(mut j) = jobs.remove(id) {
                if let Some(h) = j.handle.take() {
                    let _ = h.join();
                }
            }
            Some(*v)
        }
        Err(v) => {
            // Not this caller's type: put it back.
            *job.done.lock().unwrap_or_else(|e| e.into_inner()) = Some(v);
            None
        }
    }
}

/// Whether job `id` is running (or done and not taken yet).
pub fn busy(id: &str) -> bool {
    lock().contains_key(id)
}

/// The ids starting with `prefix` whose jobs finished (results to take).
pub fn finished_with(prefix: &str) -> Vec<String> {
    lock().iter().filter(|(k, j)| k.starts_with(prefix) && j.done.lock().map(|d| d.is_some()).unwrap_or(false)).map(|(k, _)| k.clone()).collect()
}

/// The labels of the jobs still running, oldest first.
pub fn running() -> Vec<String> {
    let jobs = lock();
    let mut v: Vec<(&Instant, &String)> = jobs.values().filter(|j| j.done.lock().map(|d| d.is_none()).unwrap_or(false)).map(|j| (&j.started, &j.label)).collect();
    v.sort();
    v.into_iter().map(|(_, l)| l.clone()).collect()
}

/// Waits (up to `limit`) for the jobs whose id starts with `prefix`, at
/// exit: a save must not be cut off.
pub fn wait(prefix: &str, limit: Duration) {
    let end = Instant::now() + limit;
    loop {
        let open = lock().iter().any(|(k, j)| k.starts_with(prefix) && j.done.lock().map(|d| d.is_none()).unwrap_or(false));
        if !open || Instant::now() >= end {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A spinner and what runs in the background, for a status bar (nothing
/// when nothing runs).
pub fn status_ui(ui: &mut egui::Ui) {
    let running = running();
    if running.is_empty() {
        return;
    }
    ui.add(egui::Spinner::new().size(12.0));
    ui.label(egui::RichText::new(running.join(" · ")).size(11.5));
}

/// Repaint now and then while jobs run, so results show (the thread also
/// asks when it finishes) and the spinner turns.
pub fn keep_painting(ctx: &egui::Context) {
    if !lock().is_empty() {
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_once() {
        let ctx = egui::Context::default();
        assert!(spawn(&ctx, "test:once", "Testing", || 41 + 1));
        assert!(!spawn(&ctx, "test:once", "Testing", || 0), "one at a time per id");
        let start = Instant::now();
        let v = loop {
            if let Some(v) = take::<i32>("test:once") {
                break v;
            }
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(v, 42);
        assert!(take::<i32>("test:once").is_none());
        assert!(!busy("test:once"));
    }

    #[test]
    fn a_panic_frees_the_id() {
        let ctx = egui::Context::default();
        assert!(spawn(&ctx, "test:panic", "Testing", || -> i32 { panic!("boom") }));
        wait("test:panic", Duration::from_secs(5));
        assert!(take::<i32>("test:panic").is_none());
        assert!(!busy("test:panic"));
        assert!(running().iter().all(|l| l != "Testing panic"));
    }

    #[test]
    fn an_untaken_result_does_not_block_the_id() {
        let ctx = egui::Context::default();
        assert!(spawn(&ctx, "test:untaken", "Testing", || 1));
        wait("test:untaken", Duration::from_secs(5));
        assert!(spawn(&ctx, "test:untaken", "Testing", || 2));
        wait("test:untaken", Duration::from_secs(5));
        assert_eq!(take::<i32>("test:untaken"), Some(2));
    }

    #[test]
    fn a_wrong_type_leaves_the_result() {
        let ctx = egui::Context::default();
        assert!(spawn(&ctx, "test:type", "Testing", || "text".to_owned()));
        wait("test:type", Duration::from_secs(5));
        assert!(take::<i32>("test:type").is_none());
        assert_eq!(take::<String>("test:type").as_deref(), Some("text"));
    }
}
