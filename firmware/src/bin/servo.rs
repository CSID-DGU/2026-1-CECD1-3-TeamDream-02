//! Servo board: watches the fan indicator lights (CdS cells) and presses buttons on command.
//! servo check             one-shot read (not while `log` runs: both would talk on the I2C bus)
//! servo log [DIR] [SEC]   run the logger: one "sample" record every SEC seconds (default: servo.conf, 60),
//!                         a "led_change" record when a light switches, a "servo" record for every press
//! servo press N [US]      ask the running logger to press with servo N (1 or 2), optionally to US instead of
//!                         the configured position; prints its answer
//! Only `log` touches the hardware, so readings and motion never overlap. A press moves one servo from its rest
//! position to the press position and back, in small steps, and switches the signal off again.
use firmware::i2c::Bus;
use firmware::logger::{self, Recorder, Stamp};
use firmware::servo_ctl::{Conf, Led, STEP_MS, check_us, classify, ramp};
use firmware::{ads1115, ina226, pwm, sys, tmp102, version};
use serde::Serialize;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SOCKET: &str = "servo.sock";
const EVENT_SERVO: &str = "servo";
const EVENT_LED_CHANGE: &str = "led_change";

#[derive(Serialize)]
struct PowerOut {
    rail_v: f64,
    input_ma: f64,
}

#[derive(Serialize)]
struct PressOut {
    /// 1 or 2
    servo: usize,
    rest_us: u32,
    press_us: u32,
    hold_ms: u64,
    /// cell voltages right before the motion; the record's own ldr values are from after it
    ldr_before_v: [Option<f64>; 2],
    /// lowest 5 V rail and highest input current seen while the servo was powered
    rail_min_v: Option<f64>,
    input_max_ma: Option<f64>,
    ok: bool,
}

#[derive(Serialize)]
struct Sample {
    /// AIN0 / AIN1 voltage; bright = low, ~3.3 V = dark OR disconnected
    ldr1_v: Option<f64>,
    ldr2_v: Option<f64>,
    /// debounced state of the two lights; null until it is known
    led_on: [Option<bool>; 2],
    /// "led_change" only: the states before the switch
    #[serde(skip_serializing_if = "Option::is_none")]
    led_before: Option<[Option<bool>; 2]>,
    /// None = channel not exported (pin low)
    pwm_on: [Option<bool>; 2],
    power: Option<PowerOut>,
    board_temp_c: Option<f64>,
    /// "servo" only
    #[serde(skip_serializing_if = "Option::is_none")]
    press: Option<PressOut>,
}

struct Sensors {
    bus: Bus,
    ina_ready: bool,
}

impl Sensors {
    fn open(errors: &mut Vec<String>) -> io::Result<Self> {
        pwm::ensure_all_off(errors);
        Ok(Self { bus: Bus::open(1)?, ina_ready: false })
    }

    fn ldr(&mut self, ch: usize, errors: &mut Vec<String>) -> Option<f64> {
        ads1115::read_v(&mut self.bus, ch).map_err(|e| errors.push(format!("ads1115 ain{ch}: {e}"))).ok()
    }

    fn ldrs(&mut self, errors: &mut Vec<String>) -> [Option<f64>; 2] {
        [self.ldr(0, errors), self.ldr(1, errors)]
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

    fn sample(&mut self, ldr: [Option<f64>; 2], led_on: [Option<bool>; 2], errors: &mut Vec<String>) -> Sample {
        let pwm_on = [0, 1].map(|ch| pwm::is_enabled(ch).map_err(|e| errors.push(format!("pwm{ch}: {e}"))).ok().flatten());
        Sample {
            ldr1_v: ldr[0],
            ldr2_v: ldr[1],
            led_on,
            led_before: None,
            pwm_on,
            power: self.power(errors),
            board_temp_c: tmp102::read_c(&mut self.bus).map_err(|e| errors.push(format!("tmp102: {e}"))).ok(),
            press: None,
        }
    }
}

/// Lowest rail and highest current while a servo is powered.
#[derive(Default)]
struct RailWatch {
    rail_min_v: Option<f64>,
    input_max_ma: Option<f64>,
}

impl RailWatch {
    fn look(&mut self, s: &mut Sensors, errors: &mut Vec<String>) {
        if let Some(p) = s.power(errors) {
            self.rail_min_v = Some(self.rail_min_v.map_or(p.rail_v, |m| m.min(p.rail_v)));
            self.input_max_ma = Some(self.input_max_ma.map_or(p.input_ma, |m| m.max(p.input_ma)));
        }
    }

    /// Keeps looking for `d`, about every 20 ms.
    fn look_for(&mut self, d: Duration, s: &mut Sensors, errors: &mut Vec<String>) {
        let end = Instant::now() + d;
        loop {
            self.look(s, errors);
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            std::thread::sleep(left.min(Duration::from_millis(20)));
        }
    }
}

/// rest -> press -> hold -> rest, then signal off (also when a step fails: `Signal` switches off when dropped).
fn motion(ch: usize, rest: u32, press: u32, hold: Duration, s: &mut Sensors, watch: &mut RailWatch, errors: &mut Vec<String>) -> io::Result<()> {
    let step = Duration::from_millis(STEP_MS);
    let signal = pwm::Signal::start(ch, rest)?;
    watch.look_for(Duration::from_millis(300), s, errors);
    for us in ramp(rest, press) {
        signal.set(us)?;
        watch.look_for(step, s, errors);
    }
    watch.look_for(hold, s, errors);
    for us in ramp(press, rest) {
        signal.set(us)?;
        watch.look_for(step, s, errors);
    }
    watch.look_for(Duration::from_millis(300), s, errors);
    Ok(())
}

struct Daemon {
    conf: Conf,
    /// Some = servo.conf is unusable; logging goes on with defaults, pressing is refused
    conf_error: Option<String>,
    sensors: Sensors,
    rec: Recorder,
    leds: [Led; 2],
    ldr: [Option<f64>; 2],
}

impl Daemon {
    fn led_on(&self) -> [Option<bool>; 2] {
        [self.leds[0].on(), self.leds[1].on()]
    }

    fn write(&mut self, event: &str, at: Stamp, mut errors: Vec<String>, fill: impl FnOnce(&mut Sample)) -> io::Result<()> {
        let mut sample = self.sensors.sample(self.ldr, self.led_on(), &mut errors);
        fill(&mut sample);
        self.rec.write(event, at, sample, errors)
    }

    /// Reads both cells; writes a "led_change" record when a light switched.
    fn poll(&mut self) -> io::Result<()> {
        let at = Stamp::now();
        let mut errors = Vec::new();
        let before = self.led_on();
        self.ldr = self.sensors.ldrs(&mut errors);
        let seen = classify(self.ldr, &self.conf);
        let mut switched = false;
        for i in 0..2 {
            // the first time a state becomes known is not a switch
            switched |= matches!(self.leds[i].update(seen[i]), Some(Some(_)));
        }
        if switched {
            eprintln!("lights {:?} -> {:?}", before, self.led_on());
            self.write(EVENT_LED_CHANGE, at, errors, |s| s.led_before = Some(before))?;
        }
        Ok(())
    }

    fn press(&mut self, servo: usize, us: Option<u32>) -> io::Result<String> {
        if let Some(e) = &self.conf_error {
            return Err(io::Error::other(format!("pressing is off until servo.conf is fixed: {e}")));
        }
        if !(1..=2).contains(&servo) {
            return Err(io::Error::other("servo must be 1 or 2"));
        }
        let ch = servo - 1;
        let (rest, press) = (self.conf.rest_us[ch], check_us(us.unwrap_or(self.conf.press_us[ch]))?);
        let hold_ms = self.conf.hold_ms;

        let at = Stamp::now();
        let mut errors = Vec::new();
        pwm::ensure_all_off(&mut errors); // one servo at a time
        let ldr_before_v = self.sensors.ldrs(&mut errors);
        let mut watch = RailWatch::default();
        let moved = motion(ch, rest, press, Duration::from_millis(hold_ms), &mut self.sensors, &mut watch, &mut errors);
        if let Err(e) = &moved {
            errors.push(format!("servo {servo}: {e}"));
        }
        std::thread::sleep(Duration::from_millis(500)); // let the light settle before the "after" reading
        self.ldr = self.sensors.ldrs(&mut errors);

        let out = PressOut {
            servo,
            rest_us: rest,
            press_us: press,
            hold_ms,
            ldr_before_v,
            rail_min_v: watch.rail_min_v,
            input_max_ma: watch.input_max_ma,
            ok: moved.is_ok(),
        };
        let reply = serde_json::to_string(&out).map_err(io::Error::other)?;
        eprintln!("press: {reply}");
        self.write(EVENT_SERVO, at, errors, |s| s.press = Some(out))?;
        moved.map(|()| reply)
    }

    /// One request per connection: "press N [US]\n" -> one JSON line.
    fn serve(&mut self, stream: UnixStream) {
        let answer = (|| {
            stream.set_nonblocking(false)?;
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line)?;
            let words: Vec<&str> = line.split_whitespace().collect();
            match words[..] {
                ["press", n] => self.press(n.parse().map_err(io::Error::other)?, None),
                ["press", n, us] => self.press(n.parse().map_err(io::Error::other)?, Some(us.parse().map_err(io::Error::other)?)),
                _ => Err(io::Error::other("use: press N [US]")),
            }
        })();
        let text = answer.unwrap_or_else(|e| serde_json::json!({ "ok": false, "error": e.to_string() }).to_string());
        let _ = (&stream).write_all(format!("{text}\n").as_bytes());
    }
}

fn conf_path() -> PathBuf {
    std::env::var("CSE_SERVO_CONF").unwrap_or_else(|_| "/etc/cse/servo.conf".into()).into()
}

fn run(dir: &Path, sample_s: Option<u64>) -> io::Result<()> {
    sys::install_stop_handler();
    let (mut conf, conf_error) = match Conf::load(&conf_path()) {
        Ok(c) => (c, None),
        Err(e) => {
            eprintln!("{e}; logging with default settings, pressing is off");
            (Conf::default(), Some(e.to_string()))
        }
    };
    if let Some(s) = sample_s {
        conf.sample_s = s;
    }
    let mut startup = Vec::new();
    let sensors = Sensors::open(&mut startup)?;
    let rec = Recorder::open(dir)?;

    let sock = dir.join(SOCKET);
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    listener.set_nonblocking(true)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o660))?;

    eprintln!(
        "servo {}: dir {}, sample every {}s, lights every {}ms (difference > {} V, else on < {} V, off > {} V), starting at seq {}",
        version::VERSION,
        dir.display(),
        conf.sample_s,
        conf.poll_ms,
        conf.led_diff_v,
        conf.led_on_below_v,
        conf.led_off_above_v,
        rec.next_seq()
    );
    let (sample_every, poll_every) = (Duration::from_secs(conf.sample_s), Duration::from_millis(conf.poll_ms));
    let mut d = Daemon { conf, conf_error, sensors, rec, leds: Default::default(), ldr: [None; 2] };

    let (mut next_poll, mut next_sample) = (Instant::now(), Instant::now());
    while !sys::stop_requested() {
        if let Ok((stream, _)) = listener.accept() {
            d.serve(stream);
        }
        let now = Instant::now();
        if now >= next_poll {
            d.poll()?;
            next_poll = logger::advance(next_poll, poll_every);
        }
        if now >= next_sample {
            let errors = std::mem::take(&mut startup);
            d.write(logger::EVENT_SAMPLE, Stamp::now(), errors, |_| {})?;
            next_sample = logger::advance(next_sample, sample_every);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&sock);
    eprintln!("servo: stopped at seq {}", d.rec.next_seq());
    Ok(())
}

/// Client side of `press`: talks to the running logger.
fn press_request(args: &[String]) -> io::Result<()> {
    let sock = std::env::var("CSE_SERVO_SOCK").unwrap_or_else(|_| format!("/var/lib/cse-servo/{SOCKET}"));
    let mut stream = UnixStream::connect(&sock).map_err(|e| io::Error::other(format!("{sock}: {e} (is the servo logger running?)")))?;
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.write_all(format!("press {}\n", args[2..].join(" ")).as_bytes())?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    println!("{}", reply.trim());
    let ok = serde_json::from_str::<serde_json::Value>(&reply).ok().and_then(|v| v["ok"].as_bool()).unwrap_or(false);
    if ok { Ok(()) } else { Err(io::Error::other("press failed")) }
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("log") => {
            let dir = logger::state_dir(args.get(2));
            let secs = args.get(3).map(|s| s.parse()).transpose().map_err(io::Error::other)?;
            run(Path::new(&dir), secs)
        }
        Some("press") if args.len() >= 3 => press_request(&args),
        Some("upload") => firmware::upload::main_from_args(&args),
        Some("check") | None => {
            let mut errors = Vec::new();
            let mut s = Sensors::open(&mut errors)?;
            let found = s.bus.scan();
            println!("fw {} | i2c: {}", version::VERSION, found.iter().map(|a| format!("0x{a:02x}")).collect::<Vec<_>>().join(" "));
            let ldr = s.ldrs(&mut errors);
            let sample = s.sample(ldr, [None; 2], &mut errors);
            println!("{}", serde_json::to_string_pretty(&sample).map_err(io::Error::other)?);
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
            eprintln!("unknown command {other}; use: check | log [DIR] [SEC] | press N [US] | upload [DIR] [once] | version");
            std::process::exit(2)
        }
    }
}
