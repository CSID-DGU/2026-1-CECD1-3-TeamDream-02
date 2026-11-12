use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;

pub struct Bus {
    file: File,
}

impl Bus {
    pub fn open(bus: u8) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/i2c-{bus}"))?;
        Ok(Self { file })
    }

    fn select(&self, addr: u8) -> io::Result<()> {
        let r = unsafe { libc::ioctl(self.file.as_raw_fd(), I2C_SLAVE as _, addr as libc::c_ulong) };
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    pub fn probe(&mut self, addr: u8) -> bool {
        let mut b = [0u8; 1];
        self.select(addr).is_ok() && self.file.read_exact(&mut b).is_ok()
    }

    pub fn write(&mut self, addr: u8, data: &[u8]) -> io::Result<()> {
        self.select(addr)?;
        self.file.write_all(data)
    }

    pub fn read_reg(&mut self, addr: u8, reg: u8, buf: &mut [u8]) -> io::Result<()> {
        self.select(addr)?;
        self.file.write_all(&[reg])?;
        self.file.read_exact(buf)
    }

    pub fn read_u16_be(&mut self, addr: u8, reg: u8) -> io::Result<u16> {
        let mut b = [0u8; 2];
        self.read_reg(addr, reg, &mut b)?;
        Ok(u16::from_be_bytes(b))
    }

    pub fn write_u16_be(&mut self, addr: u8, reg: u8, value: u16) -> io::Result<()> {
        let [hi, lo] = value.to_be_bytes();
        self.write(addr, &[reg, hi, lo])
    }

    pub fn scan(&mut self) -> Vec<u8> {
        (0x03..0x78).filter(|&a| self.probe(a)).collect()
    }
}
