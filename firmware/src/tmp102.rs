use crate::i2c::Bus;
use std::io;

pub const ADDR: u8 = 0x49;

pub fn read_c(bus: &mut Bus) -> io::Result<f64> {
    let raw = bus.read_u16_be(ADDR, 0x00)? as i16 >> 4;
    Ok(raw as f64 * 0.0625)
}
