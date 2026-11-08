//! A/B app update: <root>/slots/{a,b}, <root>/current -> slots/X, state in <root>/ota/.
use crate::fsutil::{sync_dir, write_atomic};
use crate::logger::{HEALTH_FILE, Health};
use crate::sys;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const MANIFEST: &str = "manifest.json";
pub const MANIFEST_SIG: &str = "manifest.sig";
const HEALTH_WINDOW_S: f64 = 600.0;
const HEALTHY_RECORDS: u64 = 2;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Artifact {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Manifest {
    pub version: String,
    pub build: u64,
    pub artifacts: BTreeMap<String, Artifact>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Installed {
    pub version: String,
    pub build: u64,
}

#[derive(Serialize, Deserialize, Debug)]
struct Pending {
    version: String,
    build: u64,
    old_slot: String,
    new_slot: String,
    boot_id: String,
    deadline_uptime_s: f64,
}

pub struct Config {
    pub root: PathBuf,
    pub board: String,
    pub service: String,
    pub state_dir: PathBuf,
    pub source: String,
    pub user_scope: bool,
    pub health_window_s: f64,
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

impl Config {
    /// <root>/ota.conf, `key=value` lines.
    pub fn load(root: &Path) -> io::Result<Self> {
        let text = fs::read_to_string(root.join("ota.conf"))?;
        let kv: BTreeMap<&str, &str> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.trim(), v.trim()))
            .collect();
        let get = |k: &str| kv.get(k).map(|s| s.to_string()).ok_or_else(|| err(format!("ota.conf: missing {k}")));
        Ok(Self {
            root: root.to_path_buf(),
            board: get("board")?,
            service: get("service")?,
            state_dir: get("state_dir")?.into(),
            source: get("source")?,
            user_scope: kv.get("systemctl_scope") == Some(&"user"),
            health_window_s: kv.get("health_window_s").and_then(|s| s.parse().ok()).unwrap_or(HEALTH_WINDOW_S),
        })
    }

    fn ota_dir(&self) -> PathBuf {
        self.root.join("ota")
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn verify_manifest(bytes: &[u8], sig_hex: &str, pubkey_hex: &str) -> io::Result<Manifest> {
    let pk: [u8; 32] = hex::decode(pubkey_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| err("bad public key"))?;
    let sig: [u8; 64] = hex::decode(sig_hex.trim())
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| err("bad signature encoding"))?;
    VerifyingKey::from_bytes(&pk)
        .map_err(|e| err(format!("public key: {e}")))?
        .verify_strict(bytes, &Signature::from_bytes(&sig))
        .map_err(|_| err("manifest signature INVALID"))?;
    serde_json::from_slice(bytes).map_err(|e| err(format!("manifest: {e}")))
}

fn fetch(source: &str, name: &str, dest: &Path) -> io::Result<Vec<u8>> {
    let url = format!("{}/{}", source.trim_end_matches('/'), name);
    let status = Command::new("curl")
        .args(["-fsSL", "--retry", "3", "--connect-timeout", "20", "--max-time", "600", "-o"])
        .arg(dest)
        .arg(&url)
        .status()?;
    if !status.success() {
        return Err(err(format!("download {url}: curl {status}")));
    }
    fs::read(dest)
}

fn read_json<T: for<'de> Deserialize<'de>>(p: &Path) -> io::Result<Option<T>> {
    match fs::read(p) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(|e| err(format!("{}: {e}", p.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_json<T: Serialize>(p: &Path, v: &T) -> io::Result<()> {
    write_atomic(p, &serde_json::to_vec_pretty(v).map_err(io::Error::other)?)
}

pub fn current_slot(root: &Path) -> io::Result<String> {
    let target = fs::read_link(root.join("current"))?;
    let name = target.file_name().and_then(|n| n.to_str()).ok_or_else(|| err("bad current link"))?;
    match name {
        "a" | "b" => Ok(name.to_string()),
        _ => Err(err(format!("current -> {} is not a slot", target.display()))),
    }
}

pub fn installed(root: &Path, slot: &str) -> io::Result<Installed> {
    Ok(read_json(&root.join("slots").join(slot).join("build.json"))?.unwrap_or_default())
}

fn point_current(root: &Path, slot: &str) -> io::Result<()> {
    let tmp = root.join("current.tmp");
    let _ = fs::remove_file(&tmp);
    std::os::unix::fs::symlink(format!("slots/{slot}"), &tmp)?;
    fs::rename(&tmp, root.join("current"))?;
    sync_dir(root)
}

/// Unit has Restart=always, so stopping the main process restarts it from `current`.
fn restart_service(cfg: &Config) -> io::Result<()> {
    let mut cmd = Command::new("systemctl");
    if cfg.user_scope {
        cmd.arg("--user");
    }
    let out = cmd.args(["show", "-p", "MainPID", "--value", &cfg.service]).output()?;
    let pid: i32 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
    if pid > 0 && unsafe { libc::kill(pid, libc::SIGTERM) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn lock(dir: &Path) -> io::Result<File> {
    let f = File::create(dir.join("lock"))?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(err("another ota run holds the lock"));
    }
    Ok(f)
}

fn healthy(cfg: &Config, p: &Pending) -> bool {
    matches!(
        read_json::<Health>(&cfg.state_dir.join(HEALTH_FILE)),
        Ok(Some(h)) if h.build == p.build && h.boot_id == sys::boot_id() && h.records_this_run >= HEALTHY_RECORDS
    )
}

fn bad_builds(cfg: &Config) -> io::Result<Vec<u64>> {
    Ok(read_json(&cfg.ota_dir().join("bad.json"))?.unwrap_or_default())
}

/// Waits for the new build to report healthy; rolls back to the old slot otherwise.
fn settle(cfg: &Config, mut p: Pending) -> io::Result<()> {
    let pending_path = cfg.ota_dir().join("pending.json");
    loop {
        if healthy(cfg, &p) {
            fs::remove_file(&pending_path)?;
            sync_dir(&cfg.ota_dir())?;
            eprintln!("ota: build {} ({}) healthy, kept in slot {}", p.build, p.version, p.new_slot);
            return Ok(());
        }
        let boot = sys::boot_id();
        if boot != p.boot_id {
            p.boot_id = boot;
            p.deadline_uptime_s = sys::uptime_s() + cfg.health_window_s;
            write_json(&pending_path, &p)?;
        }
        if sys::uptime_s() > p.deadline_uptime_s {
            eprintln!("ota: build {} not healthy in time, rolling back to slot {}", p.build, p.old_slot);
            point_current(&cfg.root, &p.old_slot)?;
            restart_service(cfg)?;
            let mut bad = bad_builds(cfg)?;
            bad.push(p.build);
            write_json(&cfg.ota_dir().join("bad.json"), &bad)?;
            fs::remove_file(&pending_path)?;
            sync_dir(&cfg.ota_dir())?;
            return Err(err(format!("build {} rolled back", p.build)));
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

pub fn run(cfg: &Config, pubkey_hex: &str) -> io::Result<()> {
    let ota = cfg.ota_dir();
    fs::create_dir_all(&ota)?;
    let _lock = lock(&ota)?;

    if let Some(p) = read_json::<Pending>(&ota.join("pending.json"))? {
        eprintln!("ota: resuming health check of build {}", p.build);
        return settle(cfg, p);
    }

    let cur_slot = current_slot(&cfg.root)?;
    let cur = installed(&cfg.root, &cur_slot)?;
    let dl = ota.join("download");
    let _ = fs::remove_dir_all(&dl);
    fs::create_dir_all(&dl)?;

    let mbytes = fetch(&cfg.source, MANIFEST, &dl.join(MANIFEST))?;
    let sig = fetch(&cfg.source, MANIFEST_SIG, &dl.join(MANIFEST_SIG))?;
    let m = verify_manifest(&mbytes, &String::from_utf8_lossy(&sig), pubkey_hex)?;
    if m.build <= cur.build {
        eprintln!("ota: up to date (installed build {}, offered {})", cur.build, m.build);
        return Ok(());
    }
    if bad_builds(cfg)?.contains(&m.build) {
        eprintln!("ota: build {} was rolled back before, skipping", m.build);
        return Ok(());
    }
    let art = m.artifacts.get(&cfg.board).ok_or_else(|| err(format!("manifest has no '{}'", cfg.board)))?;
    let bin = fetch(&cfg.source, &art.file, &dl.join(&art.file))?;
    if bin.len() as u64 != art.size || sha256_hex(&bin) != art.sha256 {
        return Err(err(format!("{}: size or sha256 mismatch", art.file)));
    }

    let new_slot = if cur_slot == "a" { "b" } else { "a" };
    let slot_dir = cfg.root.join("slots").join(new_slot);
    fs::create_dir_all(&slot_dir)?;
    let target = slot_dir.join(&cfg.board);
    write_atomic(&target, &bin)?;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755))?;
    write_json(&slot_dir.join("build.json"), &Installed { version: m.version.clone(), build: m.build })?;

    let p = Pending {
        version: m.version.clone(),
        build: m.build,
        old_slot: cur_slot.clone(),
        new_slot: new_slot.into(),
        boot_id: sys::boot_id(),
        deadline_uptime_s: sys::uptime_s() + cfg.health_window_s,
    };
    write_json(&ota.join("pending.json"), &p)?;
    point_current(&cfg.root, new_slot)?;
    eprintln!("ota: build {} -> {} installed in slot {new_slot}, restarting {}", cur.build, m.build, cfg.service);
    restart_service(cfg)?;
    settle(cfg, p)
}

pub fn status(cfg: &Config) -> io::Result<String> {
    let slot = current_slot(&cfg.root)?;
    let inst = installed(&cfg.root, &slot)?;
    let pending = fs::read_to_string(cfg.ota_dir().join("pending.json")).ok();
    let health = fs::read_to_string(cfg.state_dir.join(HEALTH_FILE)).ok();
    Ok(format!(
        "board {}\nslot {slot}: {} (build {})\npending: {}\nbad builds: {:?}\nhealth: {}",
        cfg.board,
        inst.version,
        inst.build,
        pending.as_deref().unwrap_or("none").trim(),
        bad_builds(cfg)?,
        health.as_deref().unwrap_or("none").trim()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn signature_roundtrip_and_tamper() {
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let pub_hex = hex::encode(key.verifying_key().to_bytes());
        let m = br#"{"version":"v1","build":3,"artifacts":{}}"#;
        let sig = hex::encode(key.sign(m).to_bytes());
        assert_eq!(verify_manifest(m, &sig, &pub_hex).unwrap().build, 3);

        let tampered = br#"{"version":"v1","build":9,"artifacts":{}}"#;
        assert!(verify_manifest(tampered, &sig, &pub_hex).is_err());
        let other = hex::encode(SigningKey::from_bytes(&[8u8; 32]).verifying_key().to_bytes());
        assert!(verify_manifest(m, &sig, &other).is_err());
    }
}
