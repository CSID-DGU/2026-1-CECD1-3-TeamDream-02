use crate::i2c::Bus;
use std::io;

pub const ADDR: u8 = 0x40;
const REG_CONFIG: u8 = 0x00;
const REG_SHUNT: u8 = 0x01;
const REG_BUS: u8 = 0x02;
const REG_MASK: u8 = 0x06;
const CONFIG_DEFAULT: u16 = 0x4127;
const SHUNT_OHM: f64 = 0.020;

#[derive(Debug, Clone, Copy)]
pub struct Reading {
    /// 5 V rail (P5V_SYS)
    pub rail_v: f64,
    /// whole-board input current on the 9 V side; do not multiply with rail_v
    pub input_a: f64,
}

/// Settings survive a Pi restart, so reset them on every start.
pub fn init(bus: &mut Bus) -> io::Result<()> {
    let mfg = bus.read_u16_be(ADDR, 0xFE)?;
    let die = bus.read_u16_be(ADDR, 0xFF)?;
    if (mfg, die) != (0x5449, 0x2260) {
        return Err(io::Error::other(format!("INA226 identity 0x{mfg:04x}/0x{die:04x}")));
    }
    bus.write_u16_be(ADDR, REG_CONFIG, CONFIG_DEFAULT)?;
    bus.write_u16_be(ADDR, REG_MASK, 0x0000)
}

pub fn read(bus: &mut Bus) -> io::Result<Reading> {
    let rail_v = bus.read_u16_be(ADDR, REG_BUS)? as f64 * 1.25e-3;
    let shunt = bus.read_u16_be(ADDR, REG_SHUNT)? as i16;
    Ok(Reading { rail_v, input_a: shunt as f64 * 2.5e-6 / SHUNT_OHM })
}
