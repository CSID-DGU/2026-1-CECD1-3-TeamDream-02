use std::{fs, io};

fn enable_path(ch: usize) -> String {
    format!("/sys/class/pwm/pwmchip0/pwm{ch}/enable")
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
