use crate::i2c::Bus;
use std::{io, thread, time::Duration};

pub const ADDR: u8 = 0x76;

#[derive(Debug, Clone, Copy)]
pub struct Reading {
    pub temp_c: f64,
    pub pressure_hpa: f64,
    pub humidity_pct: f64,
}

pub struct Bme280 {
    t: [f64; 3],
    p: [f64; 9],
    h: [f64; 6],
}

fn u16le(b: &[u8], i: usize) -> f64 { u16::from_le_bytes([b[i], b[i + 1]]) as f64 }
fn i16le(b: &[u8], i: usize) -> f64 { i16::from_le_bytes([b[i], b[i + 1]]) as f64 }

impl Bme280 {
    pub fn init(bus: &mut Bus) -> io::Result<Self> {
        let mut id = [0u8; 1];
        bus.read_reg(ADDR, 0xD0, &mut id)?;
        if id[0] != 0x60 {
            return Err(io::Error::other(format!("BME280 chip id 0x{:02x}", id[0])));
        }
        let mut c = [0u8; 26];
        let mut d = [0u8; 7];
        bus.read_reg(ADDR, 0x88, &mut c)?;
        bus.read_reg(ADDR, 0xE1, &mut d)?;

        let t = [u16le(&c, 0), i16le(&c, 2), i16le(&c, 4)];
        let mut p = [u16le(&c, 6); 9];
        for (k, v) in p.iter_mut().enumerate().skip(1) {
            *v = i16le(&c, 6 + 2 * k);
        }
        let h4 = ((d[3] as i8 as i32) << 4) | (d[4] & 0x0F) as i32;
        let h5 = ((d[5] as i8 as i32) << 4) | (d[4] >> 4) as i32;
        let h = [c[25] as f64, i16le(&d, 0), d[2] as f64, h4 as f64, h5 as f64, d[6] as i8 as f64];
        Ok(Self { t, p, h })
    }

    /// One forced-mode measurement, oversampling x1 on all three.
    pub fn read(&self, bus: &mut Bus) -> io::Result<Reading> {
        bus.write(ADDR, &[0xF2, 0x01])?;
        bus.write(ADDR, &[0xF4, 0x25])?;
        thread::sleep(Duration::from_millis(50));
        let mut r = [0u8; 8];
        bus.read_reg(ADDR, 0xF7, &mut r)?;
        let adc_p = ((r[0] as u32) << 12 | (r[1] as u32) << 4 | (r[2] as u32) >> 4) as f64;
        let adc_t = ((r[3] as u32) << 12 | (r[4] as u32) << 4 | (r[5] as u32) >> 4) as f64;
        let adc_h = ((r[6] as u32) << 8 | r[7] as u32) as f64;

        let [t1, t2, t3] = self.t;
        let v1 = (adc_t / 16384.0 - t1 / 1024.0) * t2;
        let v2 = (adc_t / 131072.0 - t1 / 8192.0).powi(2) * t3;
        let tf = v1 + v2;
        let temp_c = tf / 5120.0;

        let [p1, p2, p3, p4, p5, p6, p7, p8, p9] = self.p;
        let mut v1 = tf / 2.0 - 64000.0;
        let mut v2 = v1 * v1 * p6 / 32768.0 + v1 * p5 * 2.0;
        v2 = v2 / 4.0 + p4 * 65536.0;
        v1 = (p3 * v1 * v1 / 524288.0 + p2 * v1) / 524288.0;
        v1 = (1.0 + v1 / 32768.0) * p1;
        let mut pa = 0.0;
        if v1 != 0.0 {
            pa = (1048576.0 - adc_p - v2 / 4096.0) * 6250.0 / v1;
            pa += (p9 * pa * pa / 2147483648.0 + pa * p8 / 32768.0 + p7) / 16.0;
        }

        let [h1, h2, h3, h4, h5, h6] = self.h;
        let x = tf - 76800.0;
        let mut hum = (adc_h - (h4 * 64.0 + h5 / 16384.0 * x))
            * (h2 / 65536.0 * (1.0 + h6 / 67108864.0 * x * (1.0 + h3 / 67108864.0 * x)));
        hum = (hum * (1.0 - h1 * hum / 524288.0)).clamp(0.0, 100.0);

        Ok(Reading { temp_c, pressure_hpa: pa / 100.0, humidity_pct: hum })
    }
}
