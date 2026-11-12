use crate::i2c::Bus;
use std::{io, thread, time::Duration};

pub const ADDR: u8 = 0x48;

/// Single-shot, single-ended, +-4.096 V, 128 SPS, comparator off. AIN2/AIN3 have no sensor.
const CONFIG: [u16; 4] = [0xC383, 0xD383, 0xE383, 0xF383];

pub fn read_v(bus: &mut Bus, ch: usize) -> io::Result<f64> {
    bus.write_u16_be(ADDR, 0x01, CONFIG[ch])?;
    thread::sleep(Duration::from_millis(20));
    let raw = bus.read_u16_be(ADDR, 0x00)? as i16;
    Ok(raw as f64 * 4.096 / 32768.0)
}
