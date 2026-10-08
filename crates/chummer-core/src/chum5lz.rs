//! `.chum5lz`: Chummer's compressed save format.
//!
//! A `.chum5lz` file is the `.chum5` XML compressed with the LZMA SDK in
//! the `.lzma` ("LZMA alone") container (`LzmaHelper.CompressToLzmaFile`):
//!
//! - 5 bytes of coder properties: `lc + 9 * (lp + 5 * pb)` with lc 3,
//!   lp 0, pb 2 (`0x5D`), then the dictionary size as a little-endian u32;
//! - the uncompressed size as a little-endian i64, always -1 (unknown);
//! - the LZMA stream, closed by an end marker.
//!
//! Chummer's default preset is "Balanced" (`GlobalSettings
//! .DefaultChum5lzCompressionLevel`): a 2^24 byte dictionary, 64 fast
//! bytes and the BT4 match finder. Writing uses the same settings, so the
//! header is the one Chummer writes; the compressed bytes after it come
//! from a different encoder and differ, but decode to the same XML.
//! Reading accepts any `.lzma` file (any preset, known or unknown size).

use std::io::{Read, Write};
use std::path::Path;

use lzma_rust2::{EncodeMode, LzmaOptions, LzmaReader, LzmaWriter, MfType};

/// Whether `path` names a compressed save (by extension, as Chummer
/// decides).
pub fn is_chum5lz(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("chum5lz"))
}

/// Whether `path` is a character file chummer-rs opens: `.chumrs`, or
/// Chummer's `.chum5` or `.chum5lz`.
pub fn is_character_file(path: &Path) -> bool {
    is_chummer_file(path) || crate::chumrs::is_chumrs(path)
}

/// Whether `path` is one of Chummer5a's character files: `.chum5` or
/// `.chum5lz`.
pub fn is_chummer_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("chum5") || e.eq_ignore_ascii_case("chum5lz"))
}

/// Chummer's "Balanced" preset (`ChummerCompressionPreset.Balanced`).
fn balanced() -> LzmaOptions {
    LzmaOptions::new(1 << 24, 3, 0, 2, EncodeMode::Normal, 64, MfType::Bt4, 0)
}

/// Compress `data` into a `.chum5lz` byte stream.
pub fn compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut w = LzmaWriter::new_use_header(Vec::with_capacity(data.len() / 4 + 64), &balanced(), None).map_err(io_err)?;
    w.write_all(data)?;
    w.finish().map_err(io_err)
}

/// The most memory (KiB) the decoder may use. The header declares the
/// dictionary size and the decoder allocates it in full, so without a
/// limit a forged 13-byte header asks for 4 GiB. Chummer's largest preset
/// ("Ultra", `LzmaHelper`) uses a 2^27 byte dictionary; this allows that
/// plus the largest probability tables.
const MEM_LIMIT_KB: u32 = (1 << 27) / 1024 + 8192;

/// The most a stream may decompress to. The largest real saves are a few
/// MB of XML; a small forged stream could otherwise expand to gigabytes.
pub const MAX_DECOMPRESSED: u64 = 256 << 20;

/// Decompress a `.chum5lz` byte stream.
pub fn decompress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    if data.len() < 13 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "input .lzma is too short"));
    }
    let r = LzmaReader::new_mem_limit(data, MEM_LIMIT_KB, None).map_err(io_err)?;
    let mut out = Vec::with_capacity(data.len().saturating_mul(8).min(MAX_DECOMPRESSED as usize));
    r.take(MAX_DECOMPRESSED + 1).read_to_end(&mut out)?;
    if out.len() as u64 > MAX_DECOMPRESSED {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("decompresses to more than {MAX_DECOMPRESSED} bytes")));
    }
    Ok(out)
}

fn io_err<E: std::fmt::Display>(e: E) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("LZMA: {e}"))
}

/// The XML text of a character file, decompressed when it is a
/// `.chum5lz`, taken out of the container when it is a `.chumrs` (with
/// its mugshots put back). A UTF-8 byte order mark stays (the XML parser
/// skips it).
pub fn read_text(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    if crate::container::is_container(&bytes) {
        let (ch, _) = crate::chumrs::from_bytes(&bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        return Ok(ch.doc.to_xml_string());
    }
    text_from_bytes(&bytes, is_chum5lz(path))
}

/// The XML text of a `.chum5` (`lzma` false) or `.chum5lz` file's bytes.
pub fn text_from_bytes(bytes: &[u8], lzma: bool) -> std::io::Result<String> {
    let bytes = if lzma { decompress(bytes)? } else { bytes.to_vec() };
    String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write the XML text of a character file, compressed when `path` is a
/// `.chum5lz` (Chummer's `Character.Save`: anything not ending in
/// `.chum5` is compressed). Written to a temporary file first, then moved
/// into place.
pub fn write_text(path: &Path, text: &str) -> std::io::Result<()> {
    let bytes = if is_chum5lz(path) { compress(text.as_bytes())? } else { text.as_bytes().to_vec() };
    crate::container::atomic_write(path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_chummers_balanced_header() {
        let z = compress(b"<character />").unwrap();
        assert_eq!(&z[..13], &[0x5D, 0, 0, 0, 1, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(decompress(&z).unwrap(), b"<character />");
    }

    #[test]
    fn round_trip_large() {
        let text: String = (0..5000).map(|i| format!("<item><n>{i}</n><name>Thing {}</name></item>\n", i % 37)).collect();
        let z = compress(text.as_bytes()).unwrap();
        assert!(z.len() < text.len() / 5, "{} vs {}", z.len(), text.len());
        assert_eq!(decompress(&z).unwrap(), text.as_bytes());
        assert_eq!(decompress(&compress(b"").unwrap()).unwrap(), b"");
    }

    #[test]
    fn rejects_garbage() {
        assert!(decompress(b"\x00").is_err());
        assert!(decompress(b"2024-03-17 08:47:05, Info hello world, this is a log").is_err());
    }

    #[test]
    fn extensions() {
        assert!(is_chum5lz(Path::new("a/B.CHUM5LZ")));
        assert!(!is_chum5lz(Path::new("a/b.chum5")));
        assert!(is_character_file(Path::new("b.chum5")) && is_character_file(Path::new("b.chum5lz")));
        assert!(is_character_file(Path::new("b.chumrs")) && !is_chummer_file(Path::new("b.chumrs")));
        assert!(!is_character_file(Path::new("b.xml")));
    }
}
