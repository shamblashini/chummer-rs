//! Timing of the authority and replica locks, for finding UI stalls.
//!
//! The GUI shares [`crate::AuthorityHost::authority`] and
//! [`crate::PlayerSession::replica`] with the network tasks. A task that
//! holds a lock for long makes the UI thread wait, and the desktop then
//! says the app is "not responding". With a hook set ([`set_hook`]), every
//! wait for one of these locks and every long hold is reported. Without
//! one, a lock costs one `try_lock` more than a plain `Mutex::lock`.

use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::{Duration, Instant};

/// Holds shorter than this are not reported.
pub const HOLD_REPORT: Duration = Duration::from_millis(20);

/// What a hook is told.
#[derive(Debug, Clone, Copy)]
pub enum LockEvent {
    /// A thread waited this long for the lock (it was held elsewhere).
    Waited { lock: &'static str, wait: Duration },
    /// A thread held the lock this long (at least [`HOLD_REPORT`]).
    Held { lock: &'static str, held: Duration },
}

static HOOK: OnceLock<fn(LockEvent)> = OnceLock::new();

/// Report lock waits and long holds to `hook` (once per process; called
/// on whichever thread waited or held).
pub fn set_hook(hook: fn(LockEvent)) {
    let _ = HOOK.set(hook);
}

/// A lock guard that reports a long hold when dropped. Derefs to the
/// locked value.
pub struct Guard<'a, T> {
    guard: MutexGuard<'a, T>,
    lock: &'static str,
    since: Option<Instant>,
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        if let (Some(since), Some(hook)) = (self.since, HOOK.get()) {
            let held = since.elapsed();
            if held >= HOLD_REPORT {
                hook(LockEvent::Held { lock: self.lock, held });
            }
        }
    }
}

fn wrap<'a, T>(guard: MutexGuard<'a, T>, lock: &'static str) -> Guard<'a, T> {
    Guard { guard, lock, since: HOOK.get().map(|_| Instant::now()) }
}

/// Locks `m`, reporting the wait when another thread held it.
pub(crate) fn lock<'a, T>(m: &'a Mutex<T>, lock: &'static str) -> Guard<'a, T> {
    match m.try_lock() {
        Ok(g) => wrap(g, lock),
        Err(TryLockError::WouldBlock) => {
            let start = Instant::now();
            let g = m.lock().unwrap_or_else(|_| panic!("{lock} lock poisoned"));
            if let Some(hook) = HOOK.get() {
                hook(LockEvent::Waited { lock, wait: start.elapsed() });
            }
            wrap(g, lock)
        }
        Err(TryLockError::Poisoned(_)) => panic!("{lock} lock poisoned"),
    }
}

/// Locks `m` when it is free; `None` when another thread holds it.
pub(crate) fn try_lock<'a, T>(m: &'a Mutex<T>, lock: &'static str) -> Option<Guard<'a, T>> {
    match m.try_lock() {
        Ok(g) => Some(wrap(g, lock)),
        Err(TryLockError::WouldBlock) => None,
        Err(TryLockError::Poisoned(_)) => panic!("{lock} lock poisoned"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_lock_gives_way() {
        let m = Mutex::new(1);
        let g = lock(&m, "test");
        assert!(try_lock(&m, "test").is_none());
        drop(g);
        assert_eq!(*try_lock(&m, "test").unwrap(), 1);
    }
}
