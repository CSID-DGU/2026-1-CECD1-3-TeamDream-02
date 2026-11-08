//! Host / CI side.
//! otatool keygen <secret_out> <public_out>
//! otatool release --version V --build N --out DIR name=path [name=path ...]
//!   signing key (hex) from $OTA_SIGNING_KEY or $OTA_SIGNING_KEY_FILE
use ed25519_dalek::{Signer, SigningKey};
use firmware::ota::{Artifact, MANIFEST, MANIFEST_SIG, Manifest, sha256_hex, verify_manifest};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

fn err(m: impl Into<String>) -> io::Error {
    io::Error::other(m.into())
}

fn keygen(secret_out: &str, public_out: &str) -> io::Result<()> {
    let mut seed = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut seed)?;
    let key = SigningKey::from_bytes(&seed);
    let mut f = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(secret_out)?;
    io::Write::write_all(&mut f, hex::encode(seed).as_bytes())?;
    fs::write(public_out, hex::encode(key.verifying_key().to_bytes()))?;
    println!("secret -> {secret_out} (keep out of git; put in GitHub secret OTA_SIGNING_KEY)\npublic -> {public_out}");
    Ok(())
}

fn signing_key() -> io::Result<SigningKey> {
    let hexkey = match std::env::var("OTA_SIGNING_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => fs::read_to_string(std::env::var("OTA_SIGNING_KEY_FILE").map_err(|_| err("set OTA_SIGNING_KEY or OTA_SIGNING_KEY_FILE"))?)?,
    };
    let seed: [u8; 32] = hex::decode(hexkey.trim()).ok().and_then(|v| v.try_into().ok()).ok_or_else(|| err("bad signing key"))?;
    Ok(SigningKey::from_bytes(&seed))
}

fn release(args: &[String]) -> io::Result<()> {
    let (mut version, mut build, mut out) = (None, None, None);
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" => version = it.next().cloned(),
            "--build" => build = it.next().and_then(|s| s.parse::<u64>().ok()),
            "--out" => out = it.next().map(PathBuf::from),
            kv => files.push(kv.split_once('=').ok_or_else(|| err(format!("expected name=path, got {kv}")))?),
        }
    }
    let (version, build, out) = (
        version.ok_or_else(|| err("--version"))?,
        build.ok_or_else(|| err("--build"))?,
        out.ok_or_else(|| err("--out"))?,
    );
    fs::create_dir_all(&out)?;
    let mut artifacts = BTreeMap::new();
    for (name, path) in files {
        let data = fs::read(path)?;
        fs::write(out.join(name), &data)?;
        artifacts.insert(name.to_string(), Artifact { file: name.into(), sha256: sha256_hex(&data), size: data.len() as u64 });
    }
    let bytes = serde_json::to_vec_pretty(&Manifest { version, build, artifacts }).map_err(io::Error::other)?;
    let key = signing_key()?;
    let sig = hex::encode(key.sign(&bytes).to_bytes());
    verify_manifest(&bytes, &sig, &hex::encode(key.verifying_key().to_bytes()))?;
    fs::write(out.join(MANIFEST), &bytes)?;
    fs::write(out.join(MANIFEST_SIG), &sig)?;
    println!("{}", String::from_utf8_lossy(&bytes));
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("keygen") if args.len() == 4 => keygen(&args[2], &args[3]),
        Some("release") => release(&args[2..]),
        _ => Err(err("use: otatool keygen <secret> <public> | otatool release --version V --build N --out DIR name=path...")),
    };
    if let Err(e) = r {
        eprintln!("otatool: {e}");
        std::process::exit(1);
    }
}
