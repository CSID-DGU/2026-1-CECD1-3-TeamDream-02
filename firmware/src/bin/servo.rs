//! Stage 1: observe only. Logs the fan indicator light cells; never moves a servo.
//! servo check            one-shot read
//! servo log [DIR] [SEC]  log one record every SEC seconds (default 10)
use firmware::i2c::Bus;
use firmware::{ads1115, ina226, logger, pwm, tmp102, version};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize)]
struct PowerOut {
    rail_v: f64,
    input_ma: f64,
}

#[derive(Serialize)]
struct Sample {
    /// AIN0 / AIN1 voltage; bright = low, ~3.3 V = dark OR disconnected
    ldr1_v: Option<f64>,
    ldr2_v: Option<f64>,
    /// None = channel not exported (pin low)
    pwm_on: [Option<bool>; 2],
    power: Option<PowerOut>,
    board_temp_c: Option<f64>,
}

struct Sensors {
    bus: Bus,
    ina_ready: bool,
}

impl Sensors {
    fn open(errors: &mut Vec<String>) -> std::io::Result<Self> {
        pwm::ensure_all_off(errors);
        Ok(Self { bus: Bus::open(1)?, ina_ready: false })
    }

    fn ldr(&mut self, ch: usize, errors: &mut Vec<String>) -> Option<f64> {
        ads1115::read_v(&mut self.bus, ch).map_err(|e| errors.push(format!("ads1115 ain{ch}: {e}"))).ok()
    }

    fn power(&mut self, errors: &mut Vec<String>) -> Option<PowerOut> {
        if !self.ina_ready {
            if let Err(e) = ina226::init(&mut self.bus) {
                errors.push(format!("ina226 init: {e}"));
                return None;
            }
            self.ina_ready = true;
        }
        match ina226::read(&mut self.bus) {
            Ok(r) => Some(PowerOut { rail_v: r.rail_v, input_ma: r.input_a * 1e3 }),
            Err(e) => {
                errors.push(format!("ina226: {e}"));
                self.ina_ready = false;
                None
            }
        }
    }

    fn sample(&mut self, errors: &mut Vec<String>) -> Sample {
        let pwm_on = [0, 1].map(|ch| pwm::is_enabled(ch).map_err(|e| errors.push(format!("pwm{ch}: {e}"))).ok().flatten());
        Sample {
            ldr1_v: self.ldr(0, errors),
            ldr2_v: self.ldr(1, errors),
            pwm_on,
            power: self.power(errors),
            board_temp_c: tmp102::read_c(&mut self.bus).map_err(|e| errors.push(format!("tmp102: {e}"))).ok(),
        }
    }
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("log") => {
            let dir = logger::state_dir(args.get(2));
            let secs: u64 = args.get(3).map_or(Ok(10), |s| s.parse()).map_err(std::io::Error::other)?;
            let mut startup = Vec::new();
            let mut sensors = Sensors::open(&mut startup)?;
            logger::run(Path::new(&dir), Duration::from_secs(secs), |_, errs| {
                errs.append(&mut startup);
                sensors.sample(errs)
            })
        }
        Some("check") | None => {
            let mut errors = Vec::new();
            let mut s = Sensors::open(&mut errors)?;
            let found = s.bus.scan();
            println!("fw {} | i2c: {}", version::VERSION, found.iter().map(|a| format!("0x{a:02x}")).collect::<Vec<_>>().join(" "));
            let sample = s.sample(&mut errors);
            println!("{}", serde_json::to_string_pretty(&sample).map_err(std::io::Error::other)?);
            for e in &errors {
                println!("error: {e}");
            }
            Ok(())
        }
        Some("version") => {
            println!("{} build {}", version::VERSION, version::build());
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown command {other}; use: check | log [DIR] [SEC] | version");
            std::process::exit(2)
        }
    }
}
