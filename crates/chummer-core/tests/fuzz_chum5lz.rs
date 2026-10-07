//! The `.chum5lz` decoder on damaged and hostile input: random bytes,
//! truncated and bit-flipped streams, forged headers. It must return an
//! error (or some bytes), never panic, and never allocate what a forged
//! header asks for.

mod common;

use chummer_core::chum5lz;
use chummer_core::command;
use common::{iters, no_panic, Prng};

/// A small compressed character, and the `.chum5lz` fixture from Chummer.
fn streams() -> Vec<(String, Vec<u8>)> {
    let lz = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/chum5lz/fixer-chummer.chum5lz");
    let davis = std::fs::read(common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    vec![
        ("fixer-chummer.chum5lz".into(), std::fs::read(lz).unwrap()),
        ("Davis Jones compressed".into(), chum5lz::compress(&davis).unwrap()),
        ("empty compressed".into(), chum5lz::compress(b"").unwrap()),
    ]
}

/// A header: props, dictionary size, uncompressed size.
fn header(props: u8, dict: u32, size: i64) -> Vec<u8> {
    let mut h = vec![props];
    h.extend_from_slice(&dict.to_le_bytes());
    h.extend_from_slice(&size.to_le_bytes());
    h
}

/// Decode `bytes`; `Ok(Some(len))` when they decoded.
fn decode(label: &str, bytes: &[u8]) -> Result<Option<usize>, String> {
    let t = std::time::Instant::now();
    let r = no_panic(|| {
        let out = chum5lz::decompress(bytes)?;
        // What the sync code does with a snapshot that decodes.
        let _ = command::restore(bytes);
        Ok::<_, std::io::Error>(out.len())
    })
    .map_err(|p| format!("{label}: {p}"))?;
    let took = t.elapsed();
    if took > std::time::Duration::from_secs(5) {
        return Err(format!("{label}: took {took:?} ({r:?})"));
    }
    if let Ok(n) = r {
        if n as u64 > chum5lz::MAX_DECOMPRESSED {
            return Err(format!("{label}: decoded {n} bytes"));
        }
    }
    Ok(r.ok())
}

#[test]
fn random_bytes_never_panic() {
    let mut failures = Vec::new();
    for i in 0..iters(300) {
        let seed = common::base_seed() ^ i as u64;
        let mut rng = Prng::new(seed);
        let len = rng.below(if i % 10 == 0 { 4096 } else { 64 });
        let mut b = rng.bytes(len);
        // Half get a plausible header, so the decoder gets past it.
        if rng.chance(1, 2) && b.len() >= 13 {
            b[..13].copy_from_slice(&header(0x5D, 1 << 16, -1));
        }
        if let Err(e) = decode(&format!("seed {seed:#x} len {len}"), &b) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn mutated_streams_never_panic() {
    let mut failures = Vec::new();
    let mut partial = 0;
    for (name, z) in streams() {
        // Every truncation point near the ends, and a sample in between.
        let mut cuts: Vec<usize> = (0..z.len().min(20)).chain(z.len().saturating_sub(8)..z.len()).collect();
        let mut rng = Prng::new(common::base_seed());
        cuts.extend((0..iters(8)).map(|_| rng.below(z.len())));
        for cut in cuts {
            match decode(&format!("{name} truncated to {cut}"), &z[..cut]) {
                Err(e) => failures.push(e),
                Ok(Some(_)) if cut > 13 => partial += 1,
                Ok(_) => {}
            }
        }
        for i in 0..iters(20) {
            let seed = common::base_seed() ^ ((i as u64) << 8);
            let mut rng = Prng::new(seed);
            let mut b = z.clone();
            let mut desc = String::new();
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(b.len());
                let v = rng.next_u64() as u8;
                desc.push_str(&format!(" {at}={v:#04x}"));
                b[at] = v;
            }
            if let Err(e) = decode(&format!("{name} seed {seed:#x} flips{desc}"), &b) {
                failures.push(e);
            }
        }
    }
    // A truncated stream may decode to a prefix (the reader accepts a
    // missing end marker); `restore` then fails on the cut XML instead.
    eprintln!("truncated streams decoding to a prefix: {partial}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The header's dictionary size is allocated in full by the decoder: a
/// forged one must be refused, not honoured.
#[test]
fn forged_headers_are_refused_or_bounded() {
    let body = &streams()[1].1[13..];
    let mut failures = Vec::new();
    for dict in [0u32, 1, 4096, 1 << 24, 1 << 27, (1 << 27) + 1, 1 << 30, 0xFFFF_FFF0, u32::MAX] {
        for size in [-1i64, 0, 1, 13, i64::MAX, i64::MIN] {
            for props in [0x5Du8, 0, 224, 225, 0xFF] {
                let mut b = header(props, dict, size);
                b.extend_from_slice(body);
                let label = format!("props {props:#x} dict {dict:#x} size {size}");
                if let Err(e) = decode(&label, &b) {
                    failures.push(e);
                }
                // Bigger dictionaries than any Chummer preset are an error
                // up front (no allocation), whatever follows.
                let only = header(props, dict, size);
                if dict > (1 << 28) && props == 0x5D && size == -1 {
                    let mut just_header = only.clone();
                    just_header.extend_from_slice(&[0; 8]);
                    match chum5lz::decompress(&just_header) {
                        Err(e) if e.to_string().contains("mem") || e.to_string().contains("too large") => {}
                        other => failures.push(format!("{label}: expected a memory-limit error, got {:?}", other.map(|v| v.len()))),
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn valid_streams_still_decode() {
    for (name, z) in streams() {
        chum5lz::decompress(&z).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
