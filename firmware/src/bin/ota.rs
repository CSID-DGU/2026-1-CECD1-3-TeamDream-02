//! On the Pi: ota run [--source URL] | ota status    (root: $CSE_ROOT or /home/dreamteam/cse)
use firmware::ota::{self, Config};
use std::path::PathBuf;

const PUBKEY_HEX: &str = include_str!("../../keys/ota_signing.pub");

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(std::env::var("CSE_ROOT").unwrap_or_else(|_| "/home/dreamteam/cse".into()));
    let result = Config::load(&root).and_then(|mut cfg| match args.get(1).map(String::as_str) {
        Some("run") => {
            if let (Some("--source"), Some(src)) = (args.get(2).map(String::as_str), args.get(3)) {
                cfg.source = src.clone();
            }
            ota::run(&cfg, PUBKEY_HEX)
        }
        Some("status") => ota::status(&cfg).map(|s| println!("{s}")),
        _ => Err(std::io::Error::other("use: ota run [--source URL] | ota status")),
    });
    if let Err(e) = result {
        eprintln!("ota: {e}");
        std::process::exit(1);
    }
}
