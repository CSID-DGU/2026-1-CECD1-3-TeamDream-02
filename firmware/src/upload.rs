//! Uploads the records the logger stored to the ingest server (server/), in order, at least once.
//! The cursor (`upload.json`, next seq to send) only moves after the server answered 200, so a power cut or a lost
//! reply makes the next run send the same records again; the server drops duplicates on (board, boot_id, seq).
//! Records stay on the SD card after they are uploaded.
use crate::fsutil::write_atomic;
use crate::store::{first_seq_of, segments};
use crate::sys;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CURSOR_FILE: &str = "upload.json";
const REJECTED_FILE: &str = "upload-rejected.ndjson";
const MAX_BATCH_BYTES: usize = 512 * 1024;
const RETRY_MIN: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(900);

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

pub struct Config {
    pub url: String,
    pub token_file: PathBuf,
    pub interval: Duration,
    pub batch_records: usize,
}

impl Config {
    /// key=value lines like ota.conf: url, token_file, interval_s, batch_records.
    pub fn parse(text: &str) -> io::Result<Self> {
        let mut c = Self {
            url: String::new(),
            token_file: "/etc/cse/upload.token".into(),
            interval: Duration::from_secs(300),
            batch_records: 200,
        };
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (k, v) = line.split_once('=').ok_or_else(|| err(format!("upload.conf: bad line {line:?}")))?;
            let (k, v) = (k.trim(), v.trim());
            let num = |v: &str| v.parse::<u64>().ok().filter(|&n| n > 0).ok_or_else(|| err(format!("upload.conf: {k} must be a positive number")));
            match k {
                "url" => c.url = v.into(),
                "token_file" => c.token_file = v.into(),
                "interval_s" => c.interval = Duration::from_secs(num(v)?),
                "batch_records" => c.batch_records = num(v)? as usize,
                _ => return Err(err(format!("upload.conf: unknown key {k}"))),
            }
        }
        // the token travels in a header: HTTPS only (plain HTTP just for a server on this machine, for tests), no redirects
        let local = c.url.starts_with("http://127.0.0.1") || c.url.starts_with("http://localhost");
        if !(c.url.starts_with("https://") || local) || c.url.contains(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\\') {
            return Err(err("upload.conf: url must be https://... without spaces or quotes"));
        }
        Ok(c)
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        Self::parse(&fs::read_to_string(path).map_err(|e| err(format!("{}: {e}", path.display())))?)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Cursor {
    next_seq: u64,
}

fn load_cursor(dir: &Path) -> Cursor {
    match fs::read(dir.join(CURSOR_FILE)) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            // written atomically, so this should not happen; resending everything is safe (the server dedups)
            eprintln!("upload: {CURSOR_FILE} unreadable ({e}), starting from the beginning");
            Cursor::default()
        }),
        Err(_) => Cursor::default(),
    }
}

fn save_cursor(dir: &Path, next_seq: u64) -> io::Result<()> {
    write_atomic(&dir.join(CURSOR_FILE), &serde_json::to_vec(&Cursor { next_seq }).map_err(io::Error::other)?)
}

#[derive(Deserialize)]
struct SeqOnly {
    seq: u64,
}

struct Pending {
    seq: u64,
    line: String,
}

/// Complete lines with seq >= next_seq, in order, at most `max_records` and `max_bytes` (always at least one).
fn pending(dir: &Path, next_seq: u64, max_records: usize, max_bytes: usize) -> io::Result<Vec<Pending>> {
    let segs = segments(dir)?;
    let mut out: Vec<Pending> = Vec::new();
    let mut bytes = 0;
    for (i, seg) in segs.iter().enumerate() {
        // a segment holds the seqs below the next segment's first seq
        if segs.get(i + 1).and_then(|n| first_seq_of(n)).is_some_and(|f| f <= next_seq) {
            continue;
        }
        let data = fs::read(seg)?;
        let text = String::from_utf8_lossy(&data);
        let Some(end) = text.rfind('\n') else { continue }; // a half-written last line waits for the next round
        for line in text[..=end].lines() {
            let Some(seq) = serde_json::from_str::<SeqOnly>(line).ok().map(|s| s.seq) else { continue };
            if seq < next_seq {
                continue;
            }
            if !out.is_empty() && (out.len() >= max_records || bytes + line.len() + 1 > max_bytes) {
                return Ok(out);
            }
            bytes += line.len() + 1;
            out.push(Pending { seq, line: line.to_owned() });
        }
    }
    Ok(out)
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Stored { inserted: usize, duplicates: usize },
    /// 422: the server did not like a line; nothing was stored
    Rejected(String),
    TooLarge,
    Auth(u16),
    Retry(String),
}

#[derive(Deserialize)]
struct Stored {
    inserted: usize,
    duplicates: usize,
}

fn classify(code: u16, body: &str) -> Outcome {
    match code {
        200 => serde_json::from_str::<Stored>(body)
            .map(|s| Outcome::Stored { inserted: s.inserted, duplicates: s.duplicates })
            .unwrap_or_else(|_| Outcome::Retry("200 with an unreadable answer".into())),
        401 | 403 => Outcome::Auth(code),
        413 => Outcome::TooLarge,
        422 => Outcome::Rejected(body.trim().to_owned()),
        _ => Outcome::Retry(format!("server answered {code}")),
    }
}

fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~+/=".contains(&b))
}

/// curl reads this from stdin, so the token never shows up in the process list.
fn curl_config(url: &str, token: &str, body: &Path) -> String {
    format!(
        "url = \"{url}\"\nrequest = \"POST\"\nheader = \"Authorization: Bearer {token}\"\n\
         header = \"Content-Type: application/x-ndjson\"\ndata-binary = \"@{}\"\n",
        body.display()
    )
}

fn send(cfg: &Config, token: &str, body: &[u8]) -> io::Result<Outcome> {
    if !valid_token(token) {
        return Err(err(format!("token file {}: empty or has unexpected characters", cfg.token_file.display())));
    }
    let body_path = std::env::temp_dir().join(format!("cse-upload-{}.ndjson", std::process::id()));
    fs::write(&body_path, body)?;
    let result = (|| {
        let proto = if cfg.url.starts_with("https://") { "=https" } else { "=http" };
        let mut child = Command::new("curl")
            .args(["-q", "-sS", "--proto", proto, "--max-redirs", "0", "--connect-timeout", "20", "--max-time", "120"])
            .args(["-w", "\n%{http_code}", "-K", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child.stdin.take().ok_or_else(|| err("curl stdin"))?.write_all(curl_config(&cfg.url, token, &body_path).as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Ok(Outcome::Retry(format!("curl {}: {}", out.status, String::from_utf8_lossy(&out.stderr).trim())));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let (resp, code) = text.rsplit_once('\n').unwrap_or(("", &text));
        Ok(classify(code.trim().parse().unwrap_or(0), resp))
    })();
    let _ = fs::remove_file(&body_path);
    result
}

fn reject(dir: &Path, p: &Pending, why: &str) -> io::Result<()> {
    eprintln!("upload: seq {} rejected by the server ({why}), set aside in {REJECTED_FILE}", p.seq);
    let mut f = OpenOptions::new().create(true).append(true).open(dir.join(REJECTED_FILE))?;
    f.write_all(p.line.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_data()
}

/// Sends everything pending; returns how many records the server now has. An error means: try again later.
fn cycle(dir: &Path, cfg: &Config) -> io::Result<usize> {
    let token = fs::read_to_string(&cfg.token_file).map_err(|e| err(format!("token {}: {e}", cfg.token_file.display())))?;
    let token = token.trim();
    let mut next = load_cursor(dir).next_seq;
    let mut limit = cfg.batch_records;
    let mut sent = 0;
    loop {
        let batch = pending(dir, next, limit, MAX_BATCH_BYTES)?;
        let Some(last) = batch.last() else { return Ok(sent) };
        let body: String = batch.iter().flat_map(|p| [p.line.as_str(), "\n"]).collect();
        match send(cfg, token, body.as_bytes())? {
            Outcome::Stored { inserted, duplicates } if inserted + duplicates == batch.len() => {
                next = last.seq + 1;
                save_cursor(dir, next)?;
                sent += batch.len();
                limit = (limit * 2).min(cfg.batch_records);
            }
            Outcome::Stored { .. } => return Err(err("server's count does not match the batch")),
            Outcome::Rejected(why) if batch.len() == 1 => {
                reject(dir, &batch[0], &why)?;
                next = batch[0].seq + 1;
                save_cursor(dir, next)?;
            }
            // a bad line somewhere in the batch: halve until the bad record is alone, then set it aside
            Outcome::Rejected(_) | Outcome::TooLarge if batch.len() > 1 => limit = batch.len() / 2,
            Outcome::TooLarge => return Err(err("server refuses even a single record as too large")),
            Outcome::Auth(code) => return Err(err(format!("server answered {code}: token not accepted"))),
            Outcome::Retry(why) => return Err(err(why)),
            Outcome::Rejected(_) => unreachable!(),
        }
    }
}

/// The OTA updater only restarts the logger service; this one notices that `current` now points elsewhere and exits
/// so that systemd starts the new binary.
fn binary_replaced(argv0: &str) -> bool {
    match (fs::canonicalize(argv0), std::env::current_exe()) {
        (Ok(now), Ok(me)) => now != me,
        _ => false,
    }
}

/// `upload [DIR] [once]`: config from $CSE_UPLOAD_CONF (/etc/cse/upload.conf), DIR as for the logger.
pub fn main_from_args(args: &[String]) -> io::Result<()> {
    let dir = crate::logger::state_dir(args.get(2).filter(|a| a.as_str() != "once"));
    let dir = Path::new(&dir);
    let conf = std::env::var("CSE_UPLOAD_CONF").unwrap_or_else(|_| "/etc/cse/upload.conf".into());
    let cfg = Config::load(Path::new(&conf))?;
    if args.iter().any(|a| a == "once") {
        return cycle(dir, &cfg).map(|n| println!("uploaded {n} records"));
    }

    sys::install_stop_handler();
    let argv0 = args.first().map_or("", String::as_str);
    eprintln!("upload {}: {} -> {}, every {}s", crate::version::VERSION, dir.display(), cfg.url, cfg.interval.as_secs());
    let mut backoff = RETRY_MIN;
    loop {
        if binary_replaced(argv0) {
            eprintln!("upload: binary replaced by an update, restarting");
            return Ok(());
        }
        // TLS needs a correct clock and the board has no RTC
        let wait = if !sys::ntp_synced() {
            Duration::from_secs(15)
        } else {
            match cycle(dir, &cfg) {
                Ok(n) => {
                    if n > 0 {
                        eprintln!("upload: {n} records stored on the server");
                    }
                    backoff = RETRY_MIN;
                    cfg.interval
                }
                Err(e) => {
                    eprintln!("upload: {e}; retrying in {}s", backoff.as_secs());
                    let w = backoff;
                    backoff = (backoff * 2).min(RETRY_MAX);
                    w
                }
            }
        };
        if !sys::sleep_until(Instant::now() + wait) {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("upload-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn fill(d: &Path, n: u64) {
        let mut s = Store::open(d).unwrap();
        for _ in 0..n {
            let seq = s.next_seq();
            s.append(&format!("{{\"seq\":{seq},\"boot_id\":\"b\",\"t_unix_ms\":1}}")).unwrap();
        }
    }

    fn seqs(p: &[Pending]) -> Vec<u64> {
        p.iter().map(|p| p.seq).collect()
    }

    #[test]
    fn pending_follows_cursor_across_segments() {
        let d = tmpdir("pending");
        fill(&d, 1500); // segments data-0 (1440 lines) and data-1440
        assert_eq!(pending(&d, 0, 3, 1 << 20).unwrap().len(), 3);
        assert_eq!(seqs(&pending(&d, 1438, 5, 1 << 20).unwrap()), vec![1438, 1439, 1440, 1441, 1442]);
        assert_eq!(seqs(&pending(&d, 1498, 10, 1 << 20).unwrap()), vec![1498, 1499]);
        assert!(pending(&d, 1500, 10, 1 << 20).unwrap().is_empty());
        // the byte cap still lets one record through
        assert_eq!(pending(&d, 0, 100, 1).unwrap().len(), 1);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn pending_ignores_half_written_last_line() {
        let d = tmpdir("partial");
        fill(&d, 2);
        let seg = segments(&d).unwrap().pop().unwrap();
        OpenOptions::new().append(true).open(seg).unwrap().write_all(b"{\"seq\":2,\"boo").unwrap();
        assert_eq!(seqs(&pending(&d, 0, 10, 1 << 20).unwrap()), vec![0, 1]);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn cursor_roundtrip() {
        let d = tmpdir("cursor");
        fs::create_dir_all(&d).unwrap();
        assert_eq!(load_cursor(&d).next_seq, 0);
        save_cursor(&d, 42).unwrap();
        assert_eq!(load_cursor(&d).next_seq, 42);
        fs::write(d.join(CURSOR_FILE), b"garbage").unwrap();
        assert_eq!(load_cursor(&d).next_seq, 0);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn config_requires_https() {
        let ok = Config::parse("url=https://radon.kafu.moe/v1/records\ninterval_s=60\n").unwrap();
        assert_eq!((ok.interval.as_secs(), ok.batch_records), (60, 200));
        assert!(Config::parse("url=http://radon.kafu.moe/v1/records").is_err());
        assert!(Config::parse("url=https://x/a b").is_err());
        assert!(Config::parse("interval_s=60").is_err());
        assert!(Config::parse("url=https://x/\"\nbogus=1").is_err());
        assert!(Config::parse("url=http://127.0.0.1:8080/v1/records").is_ok());
    }

    #[test]
    fn classifies_answers() {
        assert_eq!(classify(200, r#"{"inserted":2,"duplicates":1,"last_seq":9}"#), Outcome::Stored { inserted: 2, duplicates: 1 });
        assert_eq!(classify(401, ""), Outcome::Auth(401));
        assert_eq!(classify(413, ""), Outcome::TooLarge);
        assert!(matches!(classify(422, "{\"error\":\"x\"}"), Outcome::Rejected(_)));
        assert!(matches!(classify(302, ""), Outcome::Retry(_)));
        assert!(matches!(classify(503, ""), Outcome::Retry(_)));
        assert!(matches!(classify(200, "oops"), Outcome::Retry(_)));
    }

    #[test]
    fn token_goes_into_the_stdin_config_not_argv() {
        let c = curl_config("https://h/v1/records", "abc123", Path::new("/tmp/b"));
        assert!(c.contains("Authorization: Bearer abc123") && c.contains("data-binary = \"@/tmp/b\""));
        assert!(valid_token("0f-_.~+/=Z9") && !valid_token("") && !valid_token("a b") && !valid_token("a\"b"));
    }
}
