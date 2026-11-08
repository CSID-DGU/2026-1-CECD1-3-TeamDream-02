use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

const QUERY: [u8; 9] = [0xFF, 0x01, 0x86, 0x00, 0x00, 0x00, 0x00, 0x00, 0x79];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Reply,
    AutoUpload,
}

#[derive(Debug, Clone, Copy)]
pub struct Reading {
    pub kind: Kind,
    /// rolling averages in Bq/m3
    pub avg_4h: u16,
    pub avg_24h: u16,
    pub raw: [u8; 9],
}

pub struct Zd100 {
    port: File,
    buf: Vec<u8>,
}

fn checksum_ok(f: &[u8]) -> bool {
    let sum = f[1..8].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    (!sum).wrapping_add(1) == f[8]
}

fn parse(f: &[u8; 9]) -> Option<Reading> {
    let (kind, h) = match f[1] {
        0x86 => (Kind::Reply, 2),
        0x3B => (Kind::AutoUpload, 4),
        _ => return None,
    };
    Some(Reading {
        kind,
        avg_4h: u16::from_be_bytes([f[h], f[h + 1]]),
        avg_24h: u16::from_be_bytes([f[h + 2], f[h + 3]]),
        raw: *f,
    })
}

impl Zd100 {
    pub fn open(path: &str) -> io::Result<Self> {
        let port = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)?;
        let fd = port.as_raw_fd();
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut t);
            t.c_cflag |= libc::CLOCAL | libc::CREAD;
            libc::cfsetispeed(&mut t, libc::B9600);
            libc::cfsetospeed(&mut t, libc::B9600);
            if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
                return Err(io::Error::last_os_error());
            }
            libc::tcflush(fd, libc::TCIOFLUSH);
        }
        Ok(Self { port, buf: Vec::new() })
    }

    /// Resyncs on 0xFF + valid checksum; accepts both reply and auto-upload frames.
    fn next_frame(&mut self) -> Option<Reading> {
        while self.buf.len() >= 9 {
            if self.buf[0] == 0xFF && checksum_ok(&self.buf[..9]) {
                let f: [u8; 9] = self.buf[..9].try_into().unwrap();
                self.buf.drain(..9);
                if let Some(r) = parse(&f) {
                    return Some(r);
                }
            } else {
                self.buf.remove(0);
            }
        }
        None
    }

    fn fill(&mut self, deadline: Instant) -> io::Result<bool> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        let mut pfd = libc::pollfd { fd: self.port.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, left.as_millis().min(200) as i32) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        if r > 0 {
            let mut tmp = [0u8; 64];
            match self.port.read(&mut tmp) {
                Ok(n) => self.buf.extend_from_slice(&tmp[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }

    pub fn query(&mut self, timeout: Duration) -> io::Result<Option<Reading>> {
        self.buf.clear();
        self.port.write_all(&QUERY)?;
        let deadline = Instant::now() + timeout;
        loop {
            while let Some(r) = self.next_frame() {
                if r.kind == Kind::Reply {
                    return Ok(Some(r));
                }
            }
            if !self.fill(deadline)? {
                return Ok(None);
            }
        }
    }
}
