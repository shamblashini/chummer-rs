//! Shared helpers for the robustness tests: a seeded generator, the
//! fixtures, the iteration budget and a panic catcher that reports where a
//! panic happened.

#![allow(dead_code)]

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Once;

/// SplitMix64: small, seedable, good enough to pick mutations.
#[derive(Debug, Clone)]
pub struct Prng(u64);

impl Prng {
    pub fn new(seed: u64) -> Prng {
        Prng(seed)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.next_u64() % den < num
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next_u64() as u8).collect()
    }
}

/// How many rounds a test runs: `default`, scaled by `CHUMMER_FUZZ_ITERS`
/// (a multiplier, e.g. `CHUMMER_FUZZ_ITERS=50` for a long run).
pub fn iters(default: usize) -> usize {
    let mul = std::env::var("CHUMMER_FUZZ_ITERS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1);
    default * mul.max(1)
}

/// The base seed: `CHUMMER_FUZZ_SEED`, else a fixed one so runs repeat.
pub fn base_seed() -> u64 {
    std::env::var("CHUMMER_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x00C0_FFEE_5EED)
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Every `.chum5` fixture, sorted by name.
pub fn fixtures() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(fixtures_dir())
        .unwrap()
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "chum5"))
        .collect();
    v.sort();
    v
}

/// Fixtures up to `max_bytes` (the two multi-megabyte ones make every
/// debug-build round slow).
pub fn small_fixtures(max_bytes: u64) -> Vec<PathBuf> {
    fixtures().into_iter().filter(|p| std::fs::metadata(p).is_ok_and(|m| m.len() <= max_bytes)).collect()
}

pub fn fixture_name(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
    static INSIDE: RefCell<u32> = const { RefCell::new(0) };
}

/// Run `f`, turning a panic into `Err("file:line: message")`. Panics
/// outside `no_panic` (the tests' own assertions) print as usual.
pub fn no_panic<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let prev = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if INSIDE.with(|i| *i.borrow()) == 0 {
                return prev(info);
            }
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "(non-string payload)".into());
            let mut loc = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            // A panic inside the standard library (an overflowing `sum`):
            // name the caller in chummer-core too.
            if !loc.starts_with("crates/") {
                let bt = std::backtrace::Backtrace::force_capture().to_string();
                if let Some(at) = bt.lines().map(str::trim).find(|l| l.starts_with("at ./src/") || l.starts_with("at crates/chummer-core/src/")) {
                    loc = format!("{loc} (from {})", at.trim_start_matches("at "));
                }
            }
            LAST_PANIC.with(|p| *p.borrow_mut() = Some(format!("{loc}: {msg}")));
        }));
    });
    LAST_PANIC.with(|p| *p.borrow_mut() = None);
    INSIDE.with(|i| *i.borrow_mut() += 1);
    let r = panic::catch_unwind(AssertUnwindSafe(f));
    INSIDE.with(|i| *i.borrow_mut() -= 1);
    r.map_err(|_| LAST_PANIC.with(|p| p.borrow_mut().take()).unwrap_or_else(|| "panic".into()))
}

/// Save a failing input next to the build output so it can be replayed.
pub fn save_failure(tag: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join("chummer-rs-fuzz-failures");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(tag.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>());
    let _ = std::fs::write(&p, bytes);
    p
}

/// A fresh empty directory under the system temp dir.
pub fn temp_dir(tag: &str) -> PathBuf {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("chummer-rs-fuzz-{tag}-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One engine per test binary (loading the game data is slow).
pub fn engine() -> &'static chummer_core::engine::Engine {
    static ENGINE: std::sync::OnceLock<chummer_core::engine::Engine> = std::sync::OnceLock::new();
    ENGINE.get_or_init(|| chummer_core::engine::Engine::load().expect("game data"))
}
