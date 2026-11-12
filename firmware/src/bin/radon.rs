//! radon check            one-shot read of every sensor
//! radon log [DIR] [SEC]  log one record every SEC seconds (default 60) into DIR
use firmware::bme280::Bme280;
use firmware::i2c::Bus;
use firmware::zd100::Zd100;
use firmware::{ina226, logger, tmp102, version};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const SERIAL: &str = "/dev/serial0";
const ZD100_PREHEAT_S: f64 = 180.0;
const ZD100_AVG_4H_S: f64 = 4.0 * 3600.0;
const ZD100_AVG_24H_S: f64 = 24.0 * 3600.0;
const ZD100_TIMEOUT: Duration = Duration::from_millis(2500);
const ZD100_TRIES: u8 = 3;

#[derive(Serialize)]
struct ZdOut {
    avg_4h: u16,
    avg_24h: u16,
    raw: String,
    /// queries it took to get this reply (1: the first one was answered)
    tries: u8,
}

#[derive(Serialize)]
struct EnvOut {
    temp_c: f64,
    humidity_pct: f64,
    pressure_hpa: f64,
}

#[derive(Serialize)]
struct PowerOut {
    rail_v: f64,
    input_ma: f64,
}

#[derive(Serialize)]
struct Sample {
    /// ZD100 output is not valid during the 3 min after power-up
    zd100_preheat: bool,
    /// The ZD100 has not been on for 4 h / 24 h yet, so that rolling average does not cover its whole window.
    /// Like zd100_preheat this goes by the board's uptime: the module is powered together with the board.
    zd100_avg_4h_partial: bool,
    zd100_avg_24h_partial: bool,
    zd100: Option<ZdOut>,
    bme280: Option<EnvOut>,
    power: Option<PowerOut>,
    board_temp_c: Option<f64>,
}

struct Sensors {
    bus: Bus,
    ina_ready: bool,
    bme: Option<Bme280>,
    zd: Option<Zd100>,
}

impl Sensors {
    fn open() -> std::io::Result<Self> {
        Ok(Self { bus: Bus::open(1)?, ina_ready: false, bme: None, zd: None })
    }

    fn zd100(&mut self, errors: &mut Vec<String>) -> Option<ZdOut> {
        if self.zd.is_none() {
            match Zd100::open(SERIAL) {
                Ok(z) => self.zd = Some(z),
                Err(e) => {
                    errors.push(format!("zd100 open: {e}"));
                    return None;
                }
            }
        }
        let zd = self.zd.as_mut()?;
        for tries in 1..=ZD100_TRIES {
            match zd.query(ZD100_TIMEOUT) {
                Ok(Some(r)) => {
                    return Some(ZdOut { avg_4h: r.avg_4h, avg_24h: r.avg_24h, raw: hex::encode(r.raw), tries });
                }
                Ok(None) => {}
                Err(e) => {
                    errors.push(format!("zd100: {e}"));
                    self.zd = None;
                    return None;
                }
            }
        }
        errors.push(format!("zd100: no reply in {ZD100_TRIES} tries"));
        None
    }

    fn bme280(&mut self, errors: &mut Vec<String>) -> Option<EnvOut> {
        if self.bme.is_none() {
            match Bme280::init(&mut self.bus) {
                Ok(b) => self.bme = Some(b),
                Err(e) => {
                    errors.push(format!("bme280 init: {e}"));
                    return None;
                }
            }
        }
        match self.bme.as_ref()?.read(&mut self.bus) {
            Ok(r) => Some(EnvOut { temp_c: r.temp_c, humidity_pct: r.humidity_pct, pressure_hpa: r.pressure_hpa }),
            Err(e) => {
                errors.push(format!("bme280: {e}"));
                self.bme = None;
                None
            }
        }
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

    fn board_temp(&mut self, errors: &mut Vec<String>) -> Option<f64> {
        tmp102::read_c(&mut self.bus).map_err(|e| errors.push(format!("tmp102: {e}"))).ok()
    }

    fn sample(&mut self, uptime_s: f64, errors: &mut Vec<String>) -> Sample {
        Sample {
            zd100_preheat: uptime_s < ZD100_PREHEAT_S,
            zd100_avg_4h_partial: uptime_s < ZD100_AVG_4H_S,
            zd100_avg_24h_partial: uptime_s < ZD100_AVG_24H_S,
            zd100: self.zd100(errors),
            bme280: self.bme280(errors),
            power: self.power(errors),
            board_temp_c: self.board_temp(errors),
        }
    }
}

fn check() -> std::io::Result<()> {
    let model = std::fs::read_to_string("/proc/device-tree/model").unwrap_or_default();
    println!("fw {} | {}", version::VERSION, model.trim_end_matches('\0'));
    let mut s = Sensors::open()?;
    let found = s.bus.scan();
    println!("i2c: {}", found.iter().map(|a| format!("0x{a:02x}")).collect::<Vec<_>>().join(" "));
    let mut errors = Vec::new();
    let sample = s.sample(firmware::sys::uptime_s(), &mut errors);
    println!("{}", serde_json::to_string_pretty(&sample).map_err(std::io::Error::other)?);
    for e in &errors {
        println!("error: {e}");
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("log") => {
            let dir = logger::state_dir(args.get(2));
            let secs: u64 = args.get(3).map_or(Ok(60), |s| s.parse()).map_err(std::io::Error::other)?;
            let mut sensors = Sensors::open()?;
            logger::run(Path::new(&dir), Duration::from_secs(secs), |up, errs| sensors.sample(up, errs))
        }
        Some("upload") => firmware::upload::main_from_args(&args),
        Some("check") | None => check(),
        Some("version") => {
            println!("{} build {}", version::VERSION, version::build());
            Ok(())
        }
        Some(other) => {
            eprintln!("unknown command {other}; use: check | log [DIR] [SEC] | upload [DIR] [once] | version");
            std::process::exit(2)
        }
    }
}
