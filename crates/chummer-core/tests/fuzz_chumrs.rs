//! The `.chumrs` / `.chummercampaign` container reader on damaged and
//! hostile input: random bytes, truncated and bit-flipped files, forged
//! manifests, ZIP bombs and archives with too many entries. It must
//! return an error (or a character), never panic, never take long, and
//! never decompress past its limits.

mod common;

use std::io::Write;

use chummer_core::campaign::{Campaign, Member, MemberKind};
use chummer_core::character::Character;
use chummer_core::chumrs::{self, Extras, HistoryItem};
use chummer_core::container::{self, Manifest};
use common::{iters, no_panic, Prng};

fn files() -> Vec<(String, Vec<u8>)> {
    let davis = Character::load(&common::fixtures_dir().join("Davis Jones.chum5")).unwrap();
    let barrett = Character::load(&common::fixtures_dir().join("Barrett.chum5")).unwrap();
    let extras = Extras { history: vec![HistoryItem { at: 1, author: String::new(), description: "x".into() }], ..Default::default() };
    let mut c = Campaign::new("Fuzz");
    c.add(Member::embedded(MemberKind::Npc, &barrett));
    vec![
        ("Davis Jones.chumrs".into(), chumrs::to_bytes(&davis, &extras)),
        ("Barrett.chumrs (mugshot)".into(), chumrs::to_bytes(&barrett, &Extras::default())),
        ("campaign".into(), c.to_bytes()),
    ]
}

/// Read `bytes` both as a character and as a campaign.
fn read(label: &str, bytes: &[u8]) -> Result<bool, String> {
    let t = std::time::Instant::now();
    let ok = no_panic(|| {
        let a = chumrs::from_any_bytes(bytes, false).is_ok();
        let b = Campaign::from_bytes(bytes).is_ok();
        a || b
    })
    .map_err(|p| {
        let saved = common::save_failure(label, bytes);
        format!("{label}: {p} (input in {})", saved.display())
    })?;
    let took = t.elapsed();
    if took > std::time::Duration::from_secs(5) {
        return Err(format!("{label}: took {took:?}"));
    }
    Ok(ok)
}

#[test]
fn random_bytes_never_panic() {
    let mut failures = Vec::new();
    for i in 0..iters(300) {
        let seed = common::base_seed() ^ i as u64;
        let mut rng = Prng::new(seed);
        let len = rng.below(if i % 10 == 0 { 4096 } else { 96 });
        let mut b = rng.bytes(len);
        // Most get the ZIP magic, so the reader gets past it.
        if rng.chance(3, 4) && b.len() >= 4 {
            b[..4].copy_from_slice(container::MAGIC);
        }
        if let Err(e) = read(&format!("random seed {seed:#x} len {len}"), &b) {
            failures.push(e);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn mutated_files_never_panic() {
    let mut failures = Vec::new();
    for (name, z) in files() {
        let mut cuts: Vec<usize> = (0..z.len().min(40)).chain(z.len().saturating_sub(64)..z.len()).collect();
        let mut rng = Prng::new(common::base_seed());
        cuts.extend((0..iters(16)).map(|_| rng.below(z.len())));
        for cut in cuts {
            match read(&format!("{name} truncated to {cut}"), &z[..cut]) {
                Err(e) => failures.push(e),
                Ok(true) => failures.push(format!("{name} truncated to {cut}: loaded")),
                Ok(false) => {}
            }
        }
        for i in 0..iters(40) {
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
            if let Err(e) = read(&format!("{name} seed {seed:#x} flips{desc}"), &b) {
                failures.push(e);
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A ZIP with these entries, written as given (manifest included).
fn zip_of(entries: &[(&str, &[u8])], method: zip::CompressionMethod) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(method).large_file(true);
    for (n, b) in entries {
        w.start_file(*n, o).unwrap();
        w.write_all(b).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn manifest_json(m: &Manifest) -> Vec<u8> {
    serde_json::to_vec(m).unwrap()
}

#[test]
fn forged_manifests_are_refused() {
    let base = container::decode(&chumrs::KIND, &files()[0].1).unwrap();
    let xml = base.get(chumrs::CHARACTER).unwrap().to_vec();
    let mut failures = Vec::new();
    let mut check = |label: &str, bytes: Vec<u8>, want: &str| match no_panic(|| chumrs::from_bytes(&bytes).map(|_| ()).map_err(|e| e.to_string())) {
        Err(p) => failures.push(format!("{label}: panic {p}")),
        Ok(Ok(())) => failures.push(format!("{label}: loaded")),
        Ok(Err(e)) if !e.contains(want) => failures.push(format!("{label}: {e} (wanted '{want}')")),
        Ok(Err(_)) => {}
    };
    let mut m = base.manifest.clone();
    m.schema_version = 99;
    check("version 99", zip_of(&[(container::MANIFEST, &manifest_json(&m)), (chumrs::CHARACTER, &xml)], zip::CompressionMethod::Deflated), "newer");
    let mut m = base.manifest.clone();
    m.entries.get_mut(chumrs::CHARACTER).unwrap().size = u64::MAX;
    check("huge size", zip_of(&[(container::MANIFEST, &manifest_json(&m)), (chumrs::CHARACTER, &xml)], zip::CompressionMethod::Deflated), "manifest says");
    let mut m = base.manifest.clone();
    m.format = "something else".into();
    check("other format", zip_of(&[(container::MANIFEST, &manifest_json(&m)), (chumrs::CHARACTER, &xml)], zip::CompressionMethod::Deflated), "something else");
    check("no manifest", zip_of(&[(chumrs::CHARACTER, &xml)], zip::CompressionMethod::Deflated), "manifest");
    check("manifest not JSON", zip_of(&[(container::MANIFEST, b"{nope"), (chumrs::CHARACTER, &xml)], zip::CompressionMethod::Deflated), "manifest.json");
    let mut m = base.manifest.clone();
    m.entries.clear();
    check("no character entry", zip_of(&[(container::MANIFEST, &manifest_json(&m))], zip::CompressionMethod::Deflated), "missing");
    // The character entry is not a character.
    let mut a = base.clone();
    a.put(chumrs::CHARACTER, b"<campaign/>".to_vec());
    check("not a character", container::encode(&chumrs::KIND, &a), "not a Chummer character");
    let mut a = base.clone();
    a.put(chumrs::CHARACTER, vec![0xFF, 0xFE, 0x00]);
    check("not UTF-8", container::encode(&chumrs::KIND, &a), "UTF-8");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn zip_bombs_and_many_entries_are_bounded() {
    // 300 MB of zeros deflates to a few hundred KB; the reader must stop
    // at its limit instead of inflating it all.
    let zeros = vec![0u8; 300 << 20];
    let bomb = zip_of(&[(container::MANIFEST, b"{}"), (chumrs::CHARACTER, &zeros)], zip::CompressionMethod::Deflated);
    drop(zeros);
    assert!(bomb.len() < 2 << 20, "{}", bomb.len());
    let t = std::time::Instant::now();
    let e = no_panic(|| chumrs::from_bytes(&bomb).map(|_| ()).map_err(|e| e.to_string())).unwrap().unwrap_err();
    assert!(e.contains("too large"), "{e}");
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "{:?}", t.elapsed());

    let names: Vec<String> = (0..5000).map(|i| format!("e{i}")).collect();
    let entries: Vec<(&str, &[u8])> = std::iter::once((container::MANIFEST, &b"{}"[..])).chain(names.iter().map(|n| (n.as_str(), &b""[..]))).collect();
    let many = zip_of(&entries, zip::CompressionMethod::Stored);
    let e = no_panic(|| chumrs::from_bytes(&many).map(|_| ()).map_err(|e| e.to_string())).unwrap().unwrap_err();
    assert!(e.contains("entries"), "{e}");
}
