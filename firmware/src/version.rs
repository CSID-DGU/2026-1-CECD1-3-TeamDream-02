/// Set by CI: FW_VERSION = release tag, FW_BUILD = monotonically increasing run number.
pub const VERSION: &str = match option_env!("FW_VERSION") {
    Some(v) => v,
    None => concat!("dev-", env!("CARGO_PKG_VERSION")),
};

pub fn build() -> u64 {
    option_env!("FW_BUILD").and_then(|s| s.parse().ok()).unwrap_or(0)
}
