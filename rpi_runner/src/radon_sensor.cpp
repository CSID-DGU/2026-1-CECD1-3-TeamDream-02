#include "radon_sensor.hpp"

#include <cerrno>
#include <cstring>
#include <stdexcept>

#include <fcntl.h>
#include <poll.h>
#include <termios.h>
#include <unistd.h>

// ── Q&A command frame ─────────────────────────────────────────────────────────
// FF 01 86 00 00 00 00 00 79
static constexpr uint8_t CMD_QUERY[9] = {
    0xFF, 0x01, 0x86, 0x00, 0x00, 0x00, 0x00, 0x00, 0x79
};

// ── Active-upload frame layout ────────────────────────────────────────────────
// [0]  0xFF  start
// [1]  0x3B  gas name (radon)
// [2]  0x18  unit (Bq/m³)
// [3]  0x00  decimal digits (0 → integer, resolution 1)
// [4]  conc1 high byte   (4-hour rolling avg)
// [5]  conc1 low  byte
// [6]  conc2 high byte   (24-hour rolling avg)
// [7]  conc2 low  byte
// [8]  checksum

// ── Q&A response frame layout ─────────────────────────────────────────────────
// [0]  0xFF
// [1]  0x86
// [2]  conc1 high
// [3]  conc1 low
// [4]  conc2 high
// [5]  conc2 low
// [6]  0x00  (reserved)
// [7]  0x00  (reserved)
// [8]  checksum

// ── Checksum (per datasheet) ──────────────────────────────────────────────────
// Sum bytes[1..7], then two's complement (~sum + 1)

RadonSensor::RadonSensor(std::string device, int timeout_ms)
    : device_(std::move(device)), timeout_ms_(timeout_ms) {}

RadonSensor::~RadonSensor() { close(); }

bool RadonSensor::open() {
    fd_ = ::open(device_.c_str(), O_RDWR | O_NOCTTY | O_NONBLOCK);
    if (fd_ < 0) return false;

    termios tty{};
    if (tcgetattr(fd_, &tty) != 0) { close(); return false; }

    cfsetispeed(&tty, B9600);
    cfsetospeed(&tty, B9600);

    tty.c_cflag &= ~(PARENB | CSTOPB | CSIZE | CRTSCTS);
    tty.c_cflag |=  (CS8 | CREAD | CLOCAL);
    tty.c_lflag &= ~(ICANON | ECHO | ECHOE | ISIG);
    tty.c_iflag &= ~(IXON | IXOFF | IXANY | IGNBRK | BRKINT | PARMRK | ISTRIP | INLCR | IGNCR | ICRNL);
    tty.c_oflag &= ~(OPOST | ONLCR);

    tty.c_cc[VMIN]  = 0;
    tty.c_cc[VTIME] = 0;

    if (tcsetattr(fd_, TCSANOW, &tty) != 0) { close(); return false; }
    tcflush(fd_, TCIOFLUSH);
    return true;
}

void RadonSensor::close() {
    if (fd_ >= 0) { ::close(fd_); fd_ = -1; }
}

// ── Private helpers ───────────────────────────────────────────────────────────

uint8_t RadonSensor::calc_checksum(const uint8_t* buf) const {
    uint8_t sum = 0;
    for (size_t i = 1; i < FRAME_LEN - 1; ++i) sum += buf[i];
    return static_cast<uint8_t>((~sum) + 1);
}

bool RadonSensor::verify_checksum(const uint8_t* buf) const {
    return buf[FRAME_LEN - 1] == calc_checksum(buf);
}

bool RadonSensor::read_exact(uint8_t* buf, size_t len) {
    size_t got = 0;
    pollfd pfd{ fd_, POLLIN, 0 };

    while (got < len) {
        int remaining_ms = timeout_ms_;  // simple: not subtracting elapsed time
        int ret = poll(&pfd, 1, remaining_ms);
        if (ret <= 0) return false;  // timeout or error

        ssize_t n = ::read(fd_, buf + got, len - got);
        if (n <= 0) return false;
        got += static_cast<size_t>(n);
    }
    return true;
}

bool RadonSensor::read_frame(uint8_t* buf) {
    // Sync to 0xFF start byte (discard garbage)
    for (int attempts = 0; attempts < 64; ++attempts) {
        if (!read_exact(buf, 1)) return false;
        if (buf[0] == 0xFF) break;
    }
    if (buf[0] != 0xFF) return false;

    // Read remaining 8 bytes
    return read_exact(buf + 1, FRAME_LEN - 1);
}

// ── Public API ────────────────────────────────────────────────────────────────

std::optional<RadonData> RadonSensor::query() {
    if (fd_ < 0) return std::nullopt;

    tcflush(fd_, TCIFLUSH);

    // Send Q&A command
    if (::write(fd_, CMD_QUERY, sizeof(CMD_QUERY)) != sizeof(CMD_QUERY))
        return std::nullopt;

    uint8_t buf[FRAME_LEN];
    if (!read_frame(buf)) return std::nullopt;

    // Q&A response: byte[1] must be 0x86
    if (buf[1] != 0x86) return std::nullopt;
    if (!verify_checksum(buf)) return std::nullopt;

    RadonData d;
    d.avg_4h  = static_cast<uint16_t>(buf[2] << 8 | buf[3]);
    d.avg_24h = static_cast<uint16_t>(buf[4] << 8 | buf[5]);
    return d;
}

std::optional<RadonData> RadonSensor::read_active() {
    if (fd_ < 0) return std::nullopt;

    uint8_t buf[FRAME_LEN];
    if (!read_frame(buf)) return std::nullopt;

    // Active upload frame: byte[1] must be 0x3B (radon)
    if (buf[1] != 0x3B) return std::nullopt;
    if (!verify_checksum(buf)) return std::nullopt;

    // Decimal digits in buf[3]; datasheet uses 0x00 → resolution 1
    // Kept as integer since resolution is fixed at 1 Bq/m³ for now
    RadonData d;
    d.avg_4h  = static_cast<uint16_t>(buf[4] << 8 | buf[5]);
    d.avg_24h = static_cast<uint16_t>(buf[6] << 8 | buf[7]);
    return d;
}