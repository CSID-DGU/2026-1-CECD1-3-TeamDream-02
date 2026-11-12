//! Bearer tokens. The server keeps only their SHA-256; tokens file, one per line:
//! `<name> <sha256 hex of the token> <board|reader>`. A board token uploads records as board `<name>`,
//! a reader token reads everything and uploads nothing.
use std::collections::HashMap;

use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Role {
    Board,
    Reader,
}

pub struct Tokens(HashMap<String, (String, Role)>);

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 32 && n.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Tokens {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut map = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |what: &str| format!("tokens line {}: {what}", i + 1);
            let f: Vec<&str> = line.split_whitespace().collect();
            let [name, hash, role] = f[..] else { return Err(bad("want `<name> <sha256> <board|reader>`")) };
            if !valid_name(name) {
                return Err(bad("name must be 1-32 of a-z 0-9 -"));
            }
            let hash = hash.to_ascii_lowercase();
            if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(bad("hash must be 64 hex digits (sha256sum of the token)"));
            }
            let role = match role {
                "board" => Role::Board,
                "reader" => Role::Reader,
                _ => return Err(bad("role must be board or reader")),
            };
            if map.insert(hash, (name.to_owned(), role)).is_some() {
                return Err(bad("same token listed twice"));
            }
        }
        if map.is_empty() {
            return Err("tokens file has no tokens".into());
        }
        Ok(Self(map))
    }

    /// `authorization` is the raw Authorization header value.
    pub fn check(&self, authorization: Option<&str>) -> Option<(&str, Role)> {
        let token = authorization?.strip_prefix("Bearer ")?.trim();
        if token.is_empty() {
            return None;
        }
        self.0.get(&hex::encode(Sha256::digest(token.as_bytes()))).map(|(n, r)| (n.as_str(), *r))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_hashed_tokens() {
        // sha256("secret-radon"), sha256("secret-read")
        let t = Tokens::parse(&format!(
            "# comment\nradon {} board\nteam {} reader\n",
            hex::encode(Sha256::digest(b"secret-radon")),
            hex::encode(Sha256::digest(b"secret-read")).to_uppercase(),
        ))
        .unwrap();
        assert_eq!(t.check(Some("Bearer secret-radon")), Some(("radon", Role::Board)));
        assert_eq!(t.check(Some("Bearer secret-read")), Some(("team", Role::Reader)));
        assert_eq!(t.check(Some("Bearer nope")), None);
        assert_eq!(t.check(Some("secret-radon")), None);
        assert_eq!(t.check(None), None);
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(Tokens::parse("").is_err());
        assert!(Tokens::parse("Radon abc board").is_err());
        assert!(Tokens::parse(&format!("radon {} admin", "0".repeat(64))).is_err());
    }
}
