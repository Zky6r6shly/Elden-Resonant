//! Reader for CONTROL Resonant's `.rmdtoc` / `.rmdblob` archives (COTR v3).
//!
//! The `.rmdtoc` holds a table of LZ4-compressed descriptor payloads; joined
//! together they form a "logical" TOC with archive, path, file, string and
//! chunk tables. Each file is a list of chunks living in one of the `.rmdblob`
//! files next to the TOC.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"COTR";
const VERSION: u32 = 3;
const DESCRIPTOR: usize = 0x10;
const ARCHIVE: usize = 0x18;
const PATH: usize = 0x1C;
const FILE: usize = 0x20;
const CHUNK: usize = 0x10;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

fn u32_at(b: &[u8], p: usize) -> Result<u32> {
    b.get(p..p + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| Error(format!("read past end at 0x{p:x}")))
}

fn u40_at(b: &[u8], p: usize) -> Result<u64> {
    let s = b.get(p..p + 5).ok_or_else(|| Error(format!("read past end at 0x{p:x}")))?;
    let mut v = [0u8; 8];
    v[..5].copy_from_slice(s);
    Ok(u64::from_le_bytes(v))
}

fn u64_at(b: &[u8], p: usize) -> Result<u64> {
    b.get(p..p + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| Error(format!("read past end at 0x{p:x}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub archive: u8,
    pub blob_offset: u64,
    pub stored: u32,
    pub size: u32,
    pub compressed: bool,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: String,
    pub size: u32,
    pub chunks: Vec<Chunk>,
}

#[derive(Debug)]
pub struct Archive {
    pub toc: PathBuf,
    /// Blob file paths and their expected sizes, indexed by chunk archive number.
    pub blobs: Vec<(PathBuf, u64)>,
    pub entries: Vec<Entry>,
}

fn string(logical: &[u8], pool: usize, pool_size: usize, off: usize, size: usize) -> Result<String> {
    if off > pool_size || size > pool_size - off {
        return err("string outside pool");
    }
    let raw = logical.get(pool + off..pool + off + size).ok_or(Error("string past end".into()))?;
    String::from_utf8(raw.to_vec()).map_err(|_| Error("string is not UTF-8".into()))
}

/// Rebuild the logical TOC from the descriptor payloads of a `.rmdtoc` file.
pub fn logical_toc(toc: &[u8]) -> Result<Vec<u8>> {
    if toc.len() < 0x58 || &toc[0..4] != MAGIC {
        return err("not a COTR file");
    }
    if u32_at(toc, 4)? != VERSION {
        return err(format!("unsupported COTR version {}", u32_at(toc, 4)?));
    }
    let desc_off = u32_at(toc, 0x08)? as usize;
    let desc_size = u32_at(toc, 0x0C)? as usize;
    if desc_off != 0x58 || desc_size % DESCRIPTOR != 0 || desc_off + desc_size > toc.len() {
        return err("bad descriptor table");
    }
    let mut logical = Vec::new();
    for i in 0..desc_size / DESCRIPTOR {
        let p = desc_off + i * DESCRIPTOR;
        let data_off = u40_at(toc, p + 3)? as usize;
        let decomp = u32_at(toc, p + 8)? as usize;
        let comp = u32_at(toc, p + 0xC)? as usize;
        let stored = if comp != 0 { comp } else { decomp };
        let raw = toc
            .get(data_off..data_off + stored)
            .ok_or_else(|| Error(format!("descriptor {i} past end")))?;
        if comp != 0 {
            let out = lz4_flex::block::decompress(raw, decomp).map_err(|e| Error(format!("descriptor {i}: {e}")))?;
            if out.len() != decomp {
                return err(format!("descriptor {i}: size mismatch"));
            }
            logical.extend_from_slice(&out);
        } else {
            logical.extend_from_slice(raw);
        }
    }
    Ok(logical)
}

/// Parse a `.rmdtoc` file's bytes. `dir` is the folder blobs are resolved against.
pub fn parse(toc: &[u8], toc_path: &Path) -> Result<Archive> {
    let logical = logical_toc(toc)?;
    let l = &logical;
    let archive_count = u32_at(toc, 0x14)? as usize;
    let path_off = u32_at(toc, 0x18)? as usize;
    let path_count = u32_at(toc, 0x1C)? as usize;
    let file_off = u32_at(toc, 0x20)? as usize;
    let file_count = u32_at(toc, 0x24)? as usize;
    let str_off = u32_at(toc, 0x28)? as usize;
    let str_size = u32_at(toc, 0x2C)? as usize;
    let chunk_off = u32_at(toc, 0x50)? as usize;
    let chunk_size = u32_at(toc, 0x54)? as usize;
    if u32_at(toc, 0x10)? != 0 || chunk_size % CHUNK != 0 {
        return err("bad logical layout");
    }
    for (off, n, sz) in [(0, archive_count, ARCHIVE), (path_off, path_count, PATH), (file_off, file_count, FILE), (chunk_off, chunk_size / CHUNK, CHUNK)] {
        if off + n * sz > l.len() {
            return err("table past end of logical TOC");
        }
    }
    if str_off + str_size > l.len() {
        return err("string pool past end");
    }
    let dir = toc_path.parent().unwrap_or(Path::new("."));
    let mut blobs = Vec::with_capacity(archive_count);
    for i in 0..archive_count {
        let p = i * ARCHIVE;
        let name = string(l, str_off, str_size, u32_at(l, p)? as usize, u32_at(l, p + 4)? as usize)?;
        if name.contains("..") {
            return err("archive name escapes folder");
        }
        blobs.push((dir.join(name.replace('\\', "/")), u64_at(l, p + 0x10)?));
    }
    let mut paths = Vec::with_capacity(path_count);
    for i in 0..path_count {
        let p = path_off + i * PATH;
        paths.push(string(l, str_off, str_size, u32_at(l, p + 0x14)? as usize, u32_at(l, p + 0x18)? as usize)?);
    }
    let mut entries = Vec::with_capacity(file_count);
    for i in 0..file_count {
        let p = file_off + i * FILE;
        let chunk_rel = u32_at(l, p)? as usize;
        let chunk_bytes = u32_at(l, p + 4)? as usize;
        let path_index = u32_at(l, p + 8)? as usize;
        let name = string(l, str_off, str_size, u32_at(l, p + 0xC)? as usize, u32_at(l, p + 0x10)? as usize)?;
        let total = u32_at(l, p + 0x14)?;
        if path_index >= paths.len() || chunk_bytes % CHUNK != 0 || chunk_rel + chunk_bytes > chunk_size {
            return err(format!("file {i}: bad record"));
        }
        let mut chunks = Vec::with_capacity(chunk_bytes / CHUNK);
        let mut sum = 0u64;
        for j in 0..chunk_bytes / CHUNK {
            let c = chunk_off + chunk_rel + j * CHUNK;
            let flags = l[c];
            let archive = l[c + 1];
            let decomp = u32_at(l, c + 8)?;
            let comp = u32_at(l, c + 0xC)?;
            if archive as usize >= archive_count || (flags == 0x10) != (comp != 0) {
                return err(format!("file {i}: bad chunk {j}"));
            }
            sum += decomp as u64;
            chunks.push(Chunk {
                archive,
                blob_offset: u40_at(l, c + 3)?,
                stored: if comp != 0 { comp } else { decomp },
                size: decomp,
                compressed: comp != 0,
            });
        }
        if sum != total as u64 {
            return err(format!("file {i}: chunk sizes do not add up"));
        }
        let dirname = &paths[path_index];
        let path = if dirname.is_empty() { name } else { format!("{dirname}/{name}") };
        entries.push(Entry { path, size: total, chunks });
    }
    Ok(Archive { toc: toc_path.to_path_buf(), blobs, entries })
}

pub fn open(toc_path: &Path) -> Result<Archive> {
    let bytes = std::fs::read(toc_path)?;
    parse(&bytes, toc_path)
}

impl Archive {
    /// Read one file's bytes out of the blobs.
    pub fn extract(&self, entry: &Entry) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(entry.size as usize);
        let mut open: Option<(u8, File)> = None;
        for c in &entry.chunks {
            let (blob, expected) = &self.blobs[c.archive as usize];
            if open.as_ref().map(|(a, _)| *a) != Some(c.archive) {
                let f = File::open(blob)?;
                if f.metadata()?.len() != *expected {
                    return err(format!("{} has an unexpected size", blob.display()));
                }
                open = Some((c.archive, f));
            }
            let f = &mut open.as_mut().unwrap().1;
            f.seek(SeekFrom::Start(c.blob_offset))?;
            let mut raw = vec![0u8; c.stored as usize];
            f.read_exact(&mut raw)?;
            if c.compressed {
                let d = lz4_flex::block::decompress(&raw, c.size as usize).map_err(|e| Error(e.to_string()))?;
                if d.len() != c.size as usize {
                    return err("chunk size mismatch");
                }
                out.extend_from_slice(&d);
            } else {
                out.extend_from_slice(&raw);
            }
        }
        Ok(out)
    }
}

/// Find `.rmdtoc` files under a game folder (bounded depth).
pub fn find_tocs(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(root) else { return out };
    let mut items: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    items.sort();
    for p in items {
        if p.is_dir() {
            if depth > 0 {
                out.extend(find_tocs(&p, depth - 1));
            }
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("rmdtoc")) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Build a tiny COTR v3 archive pair with the given files: (dir, name, data, compress).
    pub fn build(dir: &Path, files: &[(&str, &str, &[u8], bool)]) -> PathBuf {
        let mut blob = vec![0u8; 16];
        blob[8..16].copy_from_slice(b"IDENTITY");
        let mut strings = Vec::new();
        let mut add = |s: &str| {
            let o = strings.len() as u32;
            strings.extend_from_slice(s.as_bytes());
            (o, s.len() as u32)
        };
        let arch_name = add("data.rmdblob");
        let mut dirs: Vec<&str> = files.iter().map(|f| f.0).collect();
        dirs.sort();
        dirs.dedup();
        let dir_names: Vec<_> = dirs.iter().map(|d| add(d)).collect();
        let mut file_recs = Vec::new();
        let mut chunk_tab = Vec::new();
        let mut ordered: Vec<_> = files.to_vec();
        ordered.sort_by_key(|f| dirs.iter().position(|d| *d == f.0));
        for (d, name, data, compress) in &ordered {
            let n = add(name);
            let off = blob.len() as u64;
            let (stored, comp) = if *compress {
                let c = lz4_flex::block::compress(data);
                blob.extend_from_slice(&c);
                (c.len() as u32, c.len() as u32)
            } else {
                blob.extend_from_slice(data);
                (data.len() as u32, 0)
            };
            let _ = stored;
            let mut c = vec![if *compress { 0x10 } else { 0 }, 0, 0];
            c.extend_from_slice(&off.to_le_bytes()[..5]);
            c.extend_from_slice(&(data.len() as u32).to_le_bytes());
            c.extend_from_slice(&comp.to_le_bytes());
            let rel = chunk_tab.len() as u32;
            chunk_tab.extend_from_slice(&c);
            let pi = dirs.iter().position(|x| x == d).unwrap() as u32;
            file_recs.push((rel, pi, n, data.len() as u32));
        }
        // logical layout: archive | path | file | strings | chunk
        let mut l = Vec::new();
        l.extend_from_slice(&arch_name.0.to_le_bytes());
        l.extend_from_slice(&arch_name.1.to_le_bytes());
        l.extend_from_slice(b"IDENTITY");
        l.extend_from_slice(&(blob.len() as u64).to_le_bytes());
        let path_off = l.len() as u32;
        for (i, dn) in dir_names.iter().enumerate() {
            let first = file_recs.iter().position(|f| f.1 == i as u32).unwrap() as u32;
            let count = file_recs.iter().filter(|f| f.1 == i as u32).count() as u32;
            for v in [0xFFFF_FFFFu32, 0, 0, first, count, dn.0, dn.1] {
                l.extend_from_slice(&v.to_le_bytes());
            }
        }
        let file_off = l.len() as u32;
        for (rel, pi, n, size) in &file_recs {
            for v in [*rel, CHUNK as u32, *pi, n.0, n.1, *size, 0, 0] {
                l.extend_from_slice(&v.to_le_bytes());
            }
        }
        let str_off = l.len() as u32;
        l.extend_from_slice(&strings);
        while l.len() % 16 != 0 {
            l.push(0);
        }
        let chunk_off = l.len() as u32;
        l.extend_from_slice(&chunk_tab);

        let payload = lz4_flex::block::compress(&l);
        let mut toc = vec![0u8; 0x58 + DESCRIPTOR];
        toc[0..4].copy_from_slice(MAGIC);
        let put = |t: &mut Vec<u8>, p: usize, v: u32| t[p..p + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut toc, 4, VERSION);
        put(&mut toc, 0x08, 0x58);
        put(&mut toc, 0x0C, DESCRIPTOR as u32);
        put(&mut toc, 0x14, 1);
        put(&mut toc, 0x18, path_off);
        put(&mut toc, 0x1C, dirs.len() as u32);
        put(&mut toc, 0x20, file_off);
        put(&mut toc, 0x24, file_recs.len() as u32);
        put(&mut toc, 0x28, str_off);
        put(&mut toc, 0x2C, strings.len() as u32);
        put(&mut toc, 0x50, chunk_off);
        put(&mut toc, 0x54, chunk_tab.len() as u32);
        let data_off = toc.len() as u64;
        toc[0x58..0x5B].copy_from_slice(&[0x10, 0, 0]);
        toc[0x5B..0x60].copy_from_slice(&data_off.to_le_bytes()[..5]);
        put(&mut toc, 0x60, l.len() as u32);
        put(&mut toc, 0x64, payload.len() as u32);
        toc.extend_from_slice(&payload);

        std::fs::write(dir.join("data.rmdblob"), &blob).unwrap();
        let p = dir.join("data.rmdtoc");
        std::fs::write(&p, &toc).unwrap();
        p
    }

    #[test]
    fn reads_back_files() {
        let dir = std::env::temp_dir().join(format!("cotr-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let music = vec![7u8; 5000];
        let toc = build(&dir, &[
            ("audio/music", "theme_main.wem", &music, true),
            ("audio/sfx", "aberrant_shift.wem", b"RIFFshift", false),
        ]);
        let a = open(&toc).unwrap();
        assert_eq!(a.entries.len(), 2);
        let m = a.entries.iter().find(|e| e.path == "audio/music/theme_main.wem").unwrap();
        assert_eq!(a.extract(m).unwrap(), music);
        let s = a.entries.iter().find(|e| e.path.ends_with("aberrant_shift.wem")).unwrap();
        assert_eq!(a.extract(s).unwrap(), b"RIFFshift");
        assert_eq!(find_tocs(&dir, 2), vec![toc]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse(b"nope", Path::new("x.rmdtoc")).is_err());
        let mut t = vec![0u8; 0x60];
        t[0..4].copy_from_slice(MAGIC);
        t[4] = 2;
        assert!(parse(&t, Path::new("x.rmdtoc")).is_err());
    }
}

#[cfg(test)]
mod fixture {
    /// `FIXTURE_DIR=... FIXTURE_WEM=... cargo test write_fixture -- --ignored`
    /// writes a small fake CONTROL Resonant install for smoke tests.
    #[test]
    #[ignore]
    fn write_fixture() {
        let dir = std::path::PathBuf::from(std::env::var("FIXTURE_DIR").unwrap());
        let wem = std::fs::read(std::env::var("FIXTURE_WEM").unwrap()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        super::tests::build(&dir, &[
            ("audio/music", "main_theme.wem", &wem, true),
            ("audio/sfx/abilities", "force_push_01.wem", &wem, false),
            ("audio/sfx/weapon", "aberrant_shapeshift.wem", &wem, true),
            ("textures", "logo.tex", b"not audio", false),
        ]);
    }
}
