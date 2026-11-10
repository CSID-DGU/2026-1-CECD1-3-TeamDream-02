use crate::store::Store;
use crate::{fsutil, sys, version};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

pub const HEALTH_FILE: &str = "health.json";

/// Written after every stored record; the OTA updater uses it to accept or roll back a build.
#[derive(Serialize, Deserialize, Debug)]
pub struct Health {
    pub version: String,
    pub build: u64,
    pub boot_id: String,
    pub records_this_run: u64,
    pub t_unix_ms: i64,
}

/// Periodic measurement; the default when a record has no `event` (records before fw-v0.2.0).
pub const EVENT_SAMPLE: &str = "sample";

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    v: u32,
    seq: u64,
    /// why the record was written: "sample", or a board-specific event ("servo", "led_change")
    event: &'a str,
    fw: &'a str,
    boot_id: &'a str,
    t_unix_ms: i64,
    ntp_synced: bool,
    uptime_s: f64,
    #[serde(flatten)]
    data: T,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
}

/// The moment a record describes; taken before slow sensor reads so the timestamp is the start of the measurement.
#[derive(Clone, Copy)]
pub struct Stamp {
    pub uptime_s: f64,
    pub t_unix_ms: i64,
    pub ntp_synced: bool,
}

impl Stamp {
    pub fn now() -> Self {
        Self { uptime_s: sys::uptime_s(), t_unix_ms: sys::unix_ms(), ntp_synced: sys::ntp_synced() }
    }
}

/// Appends records to the store (fsync per record) and keeps health.json up to date.
pub struct Recorder {
    store: Store,
    health_path: std::path::PathBuf,
    boot_id: String,
    records: u64,
}

impl Recorder {
    pub fn open(dir: &Path) -> io::Result<Self> {
        Ok(Self { store: Store::open(dir)?, health_path: dir.join(HEALTH_FILE), boot_id: sys::boot_id(), records: 0 })
    }

    pub fn next_seq(&self) -> u64 {
        self.store.next_seq()
    }

    pub fn write<T: Serialize>(&mut self, event: &str, at: Stamp, data: T, errors: Vec<String>) -> io::Result<()> {
        let rec = Envelope {
            v: 1,
            seq: self.store.next_seq(),
            event,
            fw: version::VERSION,
            boot_id: &self.boot_id,
            t_unix_ms: at.t_unix_ms,
            ntp_synced: at.ntp_synced,
            uptime_s: at.uptime_s,
            data,
            errors,
        };
        let line = serde_json::to_string(&rec).map_err(io::Error::other)?;
        self.store.append(&line)?;
        if !rec.errors.is_empty() {
            eprintln!("seq {}: {}", rec.seq, rec.errors.join("; "));
        }
        self.records += 1;
        let health = Health {
            version: version::VERSION.into(),
            build: version::build(),
            boot_id: self.boot_id.clone(),
            records_this_run: self.records,
            t_unix_ms: at.t_unix_ms,
        };
        let json = serde_json::to_vec(&health).map_err(io::Error::other)?;
        if let Err(e) = fsutil::write_atomic(&self.health_path, &json) {
            eprintln!("health write: {e}");
        }
        Ok(())
    }
}

/// Next slot after `next`, skipping missed ones instead of bursting after a long stall.
pub fn advance(next: Instant, interval: Duration) -> Instant {
    let now = Instant::now();
    let mut next = next + interval;
    while next <= now {
        next += interval;
    }
    next
}

/// One "sample" record every `interval`.
pub fn run<T: Serialize>(
    dir: &Path,
    interval: Duration,
    mut sample: impl FnMut(f64, &mut Vec<String>) -> T,
) -> io::Result<()> {
    sys::install_stop_handler();
    let mut rec = Recorder::open(dir)?;
    eprintln!(
        "logger {}: dir {}, every {}s, starting at seq {}",
        version::VERSION,
        dir.display(),
        interval.as_secs(),
        rec.next_seq()
    );

    let mut next = Instant::now();
    while sys::sleep_until(next) {
        let mut errors = Vec::new();
        let at = Stamp::now();
        let data = sample(at.uptime_s, &mut errors);
        rec.write(EVENT_SAMPLE, at, data, errors)?;
        next = advance(next, interval);
    }
    eprintln!("logger: stopped at seq {}", rec.next_seq());
    Ok(())
}

pub fn state_dir(arg: Option<&String>) -> String {
    arg.cloned()
        .or_else(|| std::env::var("STATE_DIRECTORY").ok())
        .unwrap_or_else(|| "data".into())
}
