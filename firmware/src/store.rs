//! Append-only NDJSON segments: data-<first seq>.ndjson, fsync per record.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const LINES_PER_SEGMENT: usize = 1440;

pub struct Store {
    dir: PathBuf,
    file: File,
    lines: usize,
    next_seq: u64,
}

fn segments(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("data-") && n.ends_with(".ndjson"))
        })
        .collect();
    v.sort();
    Ok(v)
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Drops a partial last line left by a power loss mid-write.
fn repair(file: &mut File) -> io::Result<(usize, Option<u64>)> {
    let mut buf = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut buf)?;
    let keep = buf.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    if keep < buf.len() {
        file.set_len(keep as u64)?;
        file.sync_all()?;
    }
    let text = String::from_utf8_lossy(&buf[..keep]);
    let last_seq = text
        .lines()
        .last()
        .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .and_then(|v| v.get("seq")?.as_u64());
    file.seek(SeekFrom::End(0))?;
    Ok((text.lines().count(), last_seq))
}

impl Store {
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        if let Some(last) = segments(&dir)?.pop() {
            let mut file = OpenOptions::new().read(true).append(true).open(&last)?;
            let (lines, last_seq) = repair(&mut file)?;
            let next_seq = last_seq.map_or_else(|| first_seq_of(&last).unwrap_or(0), |s| s + 1);
            return Ok(Self { dir, file, lines, next_seq });
        }
        let file = Self::create_segment(&dir, 0)?;
        Ok(Self { dir, file, lines: 0, next_seq: 0 })
    }

    fn create_segment(dir: &Path, seq: u64) -> io::Result<File> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(dir.join(format!("data-{seq:012}.ndjson")))?;
        sync_dir(dir)?;
        Ok(file)
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn append(&mut self, line: &str) -> io::Result<()> {
        if self.lines >= LINES_PER_SEGMENT {
            self.file = Self::create_segment(&self.dir, self.next_seq)?;
            self.lines = 0;
        }
        let mut buf = Vec::with_capacity(line.len() + 1);
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
        self.file.write_all(&buf)?;
        self.file.sync_data()?;
        self.lines += 1;
        self.next_seq += 1;
        Ok(())
    }
}

fn first_seq_of(p: &Path) -> Option<u64> {
    p.file_stem()?.to_str()?.strip_prefix("data-")?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("store-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn resumes_seq_and_drops_partial_line() {
        let d = tmpdir("resume");
        let mut s = Store::open(&d).unwrap();
        for _ in 0..3 {
            let seq = s.next_seq();
            s.append(&format!("{{\"seq\":{seq}}}")).unwrap();
        }
        drop(s);
        let seg = segments(&d).unwrap().pop().unwrap();
        OpenOptions::new().append(true).open(&seg).unwrap().write_all(b"{\"seq\":3,\"trunc").unwrap();

        let s = Store::open(&d).unwrap();
        assert_eq!(s.next_seq(), 3);
        assert_eq!(fs::read_to_string(&seg).unwrap().lines().count(), 3);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rotates_segments() {
        let d = tmpdir("rotate");
        let mut s = Store::open(&d).unwrap();
        for _ in 0..LINES_PER_SEGMENT + 1 {
            let seq = s.next_seq();
            s.append(&format!("{{\"seq\":{seq}}}")).unwrap();
        }
        let segs = segments(&d).unwrap();
        assert_eq!(segs.len(), 2);
        assert!(segs[1].ends_with(format!("data-{LINES_PER_SEGMENT:012}.ndjson")));
        drop(s);
        assert_eq!(Store::open(&d).unwrap().next_seq(), LINES_PER_SEGMENT as u64 + 1);
        fs::remove_dir_all(&d).unwrap();
    }
}
