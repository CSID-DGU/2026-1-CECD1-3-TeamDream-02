//! Servo board logic that needs no hardware: settings, the indicator-light state and the motion ramp.
use std::io;
use std::path::Path;

/// Hard limits of a servo signal; no setting can widen them.
pub const MIN_US: u32 = 1300;
pub const MAX_US: u32 = 1700;
/// The signal moves in steps of STEP_US every STEP_MS, never in one jump.
pub const STEP_US: u32 = 20;
pub const STEP_MS: u64 = 60;
const MAX_HOLD_MS: u64 = 5000;
/// Consecutive readings that must agree before a light counts as switched.
const DEBOUNCE: u32 = 3;

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

#[derive(Debug, Clone, PartialEq)]
pub struct Conf {
    /// seconds between "sample" records (same as the radon board, so the two line up by time)
    pub sample_s: u64,
    /// milliseconds between light-cell readings
    pub poll_ms: u64,
    /// Normally exactly one of the two lights is on, and a cell reads low when its light is on: when the two
    /// cell voltages differ by more than this, the lower one is the lit light and the other one is off.
    pub led_diff_v: f64,
    /// When they do not differ that much (both on, or both off), each cell is judged by itself:
    /// on below this voltage ...
    pub led_on_below_v: f64,
    /// ... off above this one; in between the previous state stands
    pub led_off_above_v: f64,
    pub rest_us: [u32; 2],
    pub press_us: [u32; 2],
    pub hold_ms: u64,
}

impl Default for Conf {
    fn default() -> Self {
        Self {
            sample_s: 60,
            poll_ms: 1000,
            led_diff_v: 0.5,
            led_on_below_v: 2.0,
            led_off_above_v: 2.4,
            rest_us: [1500; 2],
            press_us: [1600; 2],
            hold_ms: 500,
        }
    }
}

pub fn check_us(us: u32) -> io::Result<u32> {
    if (MIN_US..=MAX_US).contains(&us) { Ok(us) } else { Err(err(format!("{us} us is outside {MIN_US}..{MAX_US}"))) }
}

impl Conf {
    /// key=value lines like ota.conf; every key is optional.
    pub fn parse(text: &str) -> io::Result<Self> {
        let mut c = Self::default();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let (k, v) = line.split_once('=').ok_or_else(|| err(format!("servo.conf: bad line {line:?}")))?;
            let (k, v) = (k.trim(), v.trim());
            let bad = || err(format!("servo.conf: bad value for {k}"));
            let int = || v.parse::<u64>().map_err(|_| bad());
            let us = || v.parse::<u32>().map_err(|_| bad()).and_then(check_us);
            match k {
                "sample_s" => c.sample_s = int()?,
                "poll_ms" => c.poll_ms = int()?,
                "led_diff_v" => c.led_diff_v = v.parse().map_err(|_| bad())?,
                "led_on_below_v" => c.led_on_below_v = v.parse().map_err(|_| bad())?,
                "led_off_above_v" => c.led_off_above_v = v.parse().map_err(|_| bad())?,
                "sv1_rest_us" => c.rest_us[0] = us()?,
                "sv2_rest_us" => c.rest_us[1] = us()?,
                "sv1_press_us" => c.press_us[0] = us()?,
                "sv2_press_us" => c.press_us[1] = us()?,
                "hold_ms" => c.hold_ms = int()?,
                _ => return Err(err(format!("servo.conf: unknown key {k}"))),
            }
        }
        if c.sample_s == 0 || !(100..=60_000).contains(&c.poll_ms) || c.hold_ms > MAX_HOLD_MS {
            return Err(err(format!("servo.conf: sample_s >= 1, poll_ms 100..60000, hold_ms <= {MAX_HOLD_MS}")));
        }
        if !(c.led_on_below_v < c.led_off_above_v) || !(c.led_diff_v > 0.0) {
            return Err(err("servo.conf: led_on_below_v must be below led_off_above_v, led_diff_v above 0"));
        }
        Ok(c)
    }

    /// A missing file means defaults.
    pub fn load(path: &Path) -> io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(err(format!("{}: {e}", path.display()))),
        }
    }
}

/// Signal values from `from` (exclusive) to `to` (inclusive), STEP_US apart.
pub fn ramp(from: u32, to: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut u = from;
    while u != to {
        u = if to > u { (u + STEP_US).min(to) } else { u.saturating_sub(STEP_US).max(to) };
        out.push(u);
    }
    out
}

/// What one reading of the two cells (None = read failed) says about the two lights; None = nothing.
pub fn classify(v: [Option<f64>; 2], c: &Conf) -> [Option<bool>; 2] {
    if let [Some(a), Some(b)] = v {
        if b - a > c.led_diff_v {
            return [Some(true), Some(false)];
        }
        if a - b > c.led_diff_v {
            return [Some(false), Some(true)];
        }
    }
    v.map(|v| match v {
        Some(v) if v < c.led_on_below_v => Some(true),
        Some(v) if v > c.led_off_above_v => Some(false),
        _ => None, // between the thresholds or unreadable
    })
}

/// One indicator light, debounced.
#[derive(Default)]
pub struct Led {
    on: Option<bool>,
    candidate: Option<bool>,
    seen: u32,
}

impl Led {
    /// None until enough readings agreed once.
    pub fn on(&self) -> Option<bool> {
        self.on
    }

    /// Takes what one reading says (see `classify`). Returns the previous state when the light switched.
    pub fn update(&mut self, now: Option<bool>) -> Option<Option<bool>> {
        if now.is_none() || now == self.on {
            self.candidate = None;
            return None;
        }
        if self.candidate == now {
            self.seen += 1;
        } else {
            self.candidate = now;
            self.seen = 1;
        }
        if self.seen < DEBOUNCE {
            return None;
        }
        self.candidate = None;
        Some(std::mem::replace(&mut self.on, now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conf_defaults_and_limits() {
        assert_eq!(Conf::parse("").unwrap(), Conf::default());
        let c = Conf::parse("# site\nsv1_press_us = 1400\nsv2_rest_us=1550\nhold_ms=300\nsample_s=30\n").unwrap();
        assert_eq!((c.press_us, c.rest_us, c.hold_ms, c.sample_s), ([1400, 1600], [1500, 1550], 300, 30));
        assert!(Conf::parse("sv1_press_us=1800").is_err());
        assert!(Conf::parse("sv2_rest_us=1200").is_err());
        assert!(Conf::parse("hold_ms=60000").is_err());
        assert!(Conf::parse("led_on_below_v=2.5").is_err());
        assert!(Conf::parse("led_diff_v=0").is_err());
        assert!(Conf::parse("bogus=1").is_err());
        assert!(Conf::parse("poll_ms=10").is_err());
    }

    #[test]
    fn ramp_never_jumps_and_ends_exactly() {
        assert_eq!(ramp(1500, 1560), vec![1520, 1540, 1560]);
        assert_eq!(ramp(1500, 1450), vec![1480, 1460, 1450]);
        assert!(ramp(1500, 1500).is_empty());
        let r = ramp(1300, 1700);
        assert_eq!((r.len(), r[r.len() - 1]), (20, 1700));
        assert!(r.windows(2).all(|w| w[1] - w[0] <= STEP_US));
    }

    #[test]
    fn lights_are_told_apart_by_the_difference_first() {
        let c = Conf::default();
        // normal case, one light on: the clearly lower cell is the lit one, whatever the absolute level
        assert_eq!(classify([Some(1.45), Some(2.83)], &c), [Some(true), Some(false)]);
        assert_eq!(classify([Some(2.9), Some(2.3)], &c), [Some(false), Some(true)]);
        assert_eq!(classify([Some(2.2), Some(3.2)], &c), [Some(true), Some(false)]);
        // about equal: both on, both off, or undecided, by the absolute thresholds
        assert_eq!(classify([Some(1.4), Some(1.6)], &c), [Some(true), Some(true)]);
        assert_eq!(classify([Some(3.1), Some(3.2)], &c), [Some(false), Some(false)]);
        assert_eq!(classify([Some(2.2), Some(2.3)], &c), [None, None]);
        // one cell unreadable: the other is judged by itself
        assert_eq!(classify([None, Some(1.2)], &c), [None, Some(true)]);
        assert_eq!(classify([None, None], &c), [None, None]);
    }

    #[test]
    fn led_switches_only_after_agreeing_readings() {
        let (on, off) = (Some(true), Some(false));
        let mut led = Led::default();
        // start-up: three "on" readings establish the state, reported as a switch from unknown
        assert_eq!(led.update(on), None);
        assert_eq!(led.update(on), None);
        assert_eq!(led.update(on), Some(None));
        assert_eq!(led.on(), on);
        // one "off" flicker and readings that say nothing change nothing
        assert_eq!(led.update(off), None);
        assert_eq!(led.update(on), None);
        assert_eq!(led.update(None), None);
        assert_eq!(led.on(), on);
        // three "off" readings in a row: switched, previous state was on
        assert_eq!(led.update(off), None);
        assert_eq!(led.update(off), None);
        assert_eq!(led.update(off), Some(on));
        assert_eq!(led.on(), off);
        // an interrupted run starts counting again
        assert_eq!(led.update(on), None);
        assert_eq!(led.update(on), None);
        assert_eq!(led.update(None), None);
        assert_eq!(led.update(on), None);
        assert_eq!(led.on(), off);
    }
}
