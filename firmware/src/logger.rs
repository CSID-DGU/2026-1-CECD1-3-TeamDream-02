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

#[derive(Serialize)]
struct Envelope<'a, T: Serialize> {
    v: u32,
    seq: u64,
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

pub fn run<T: Serialize>(
    dir: &Path,
    interval: Duration,
    mut sample: impl FnMut(f64, &mut Vec<String>) -> T,
) -> io::Result<()> {
    sys::install_stop_handler();
    let boot_id = sys::boot_id();
    let mut store = Store::open(dir)?;
    let health_path = dir.join(HEALTH_FILE);
    eprintln!(
        "logger {}: dir {}, every {}s, starting at seq {}",
        version::VERSION,
        dir.display(),
        interval.as_secs(),
        store.next_seq()
    );

    let mut records = 0u64;
    let mut next = Instant::now();
    while sys::sleep_until(next) {
        let mut errors = Vec::new();
        let uptime_s = sys::uptime_s();
        let t_unix_ms = sys::unix_ms();
        let ntp_synced = sys::ntp_synced();
        let data = sample(uptime_s, &mut errors);
        let rec = Envelope {
            v: 1,
            seq: store.next_seq(),
            fw: version::VERSION,
            boot_id: &boot_id,
            t_unix_ms,
            ntp_synced,
            uptime_s,
            data,
            errors,
        };
        let line = serde_json::to_string(&rec).map_err(io::Error::other)?;
        store.append(&line)?;
        if !rec.errors.is_empty() {
            eprintln!("seq {}: {}", rec.seq, rec.errors.join("; "));
        }
        records += 1;
        let health = Health {
            version: version::VERSION.into(),
            build: version::build(),
            boot_id: boot_id.clone(),
            records_this_run: records,
            t_unix_ms,
        };
        let json = serde_json::to_vec(&health).map_err(io::Error::other)?;
        if let Err(e) = fsutil::write_atomic(&health_path, &json) {
            eprintln!("health write: {e}");
        }

        // Skip missed slots instead of bursting after a long stall.
        let now = Instant::now();
        next += interval;
        while next <= now {
            next += interval;
        }
    }
    eprintln!("logger: stopped at seq {}", store.next_seq());
    Ok(())
}

pub fn state_dir(arg: Option<&String>) -> String {
    arg.cloned()
        .or_else(|| std::env::var("STATE_DIRECTORY").ok())
        .unwrap_or_else(|| "data".into())
}
