use crate::servo_ctl::check_us;
use std::{fs, io};

/// 50 Hz servo frame
const PERIOD_NS: u32 = 20_000_000;

fn chan(ch: usize) -> String {
    format!("/sys/class/pwm/pwmchip0/pwm{ch}")
}

fn enable_path(ch: usize) -> String {
    format!("{}/enable", chan(ch))
}

/// None = channel not exported, which leaves the pin low.
pub fn is_enabled(ch: usize) -> io::Result<Option<bool>> {
    match fs::read_to_string(enable_path(ch)) {
        Ok(s) => Ok(Some(s.trim() == "1")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn disable(ch: usize) -> io::Result<()> {
    fs::write(enable_path(ch), "0")
}

/// Start-up rule: both servo signals off before anything else.
pub fn ensure_all_off(errors: &mut Vec<String>) {
    for ch in 0..2 {
        match is_enabled(ch) {
            Ok(Some(true)) => {
                if let Err(e) = disable(ch) {
                    errors.push(format!("pwm{ch} is ON and could not be turned off: {e}"));
                }
            }
            Ok(_) => {}
            Err(e) => errors.push(format!("pwm{ch} state: {e}")),
        }
    }
}

/// A running servo signal. Dropping it switches the signal off, also on an error or a panic.
/// The channel must be exported and writable for this user (the systemd unit does that as root).
pub struct Signal {
    ch: usize,
}

impl Signal {
    pub fn start(ch: usize, us: u32) -> io::Result<Self> {
        let us = check_us(us)?;
        if is_enabled(ch)?.is_none() {
            return Err(io::Error::other(format!("pwm{ch} is not exported")));
        }
        let s = Self { ch }; // from here on, any failure switches the signal off again
        fs::write(format!("{}/period", chan(ch)), PERIOD_NS.to_string())?;
        s.set(us)?;
        fs::write(enable_path(ch), "1")?;
        Ok(s)
    }

    pub fn set(&self, us: u32) -> io::Result<()> {
        fs::write(format!("{}/duty_cycle", chan(self.ch)), (check_us(us)? * 1000).to_string())
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        if let Err(e) = disable(self.ch) {
            eprintln!("pwm{}: could not switch the signal off: {e}", self.ch);
        }
    }
}
