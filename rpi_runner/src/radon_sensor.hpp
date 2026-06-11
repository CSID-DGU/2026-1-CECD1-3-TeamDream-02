#pragma once

#include <cstdint>
#include <optional>
#include <string>

// ZD100 Radon Detection Module — Winsen Electronics
// UART: 9600 baud, 8N1, 3.3V TTL
// Protocol: 9-byte frames, checksum = two's complement of sum of bytes[1..7]

struct RadonData {
    uint16_t avg_4h;   // Bq/m³, 4-hour rolling average
    uint16_t avg_24h;  // Bq/m³, 24-hour rolling average
};

class RadonSensor {
public:
    explicit RadonSensor(std::string device, int timeout_ms = 5000);
    ~RadonSensor();

    // Non-copyable
    RadonSensor(const RadonSensor&) = delete;
    RadonSensor& operator=(const RadonSensor&) = delete;

    bool open();
    void close();
    bool is_open() const { return fd_ >= 0; }

    // Q&A mode: actively query the sensor (FF 01 86 ... 79)
    // Returns nullopt on timeout or checksum error
    std::optional<RadonData> query();

    // Active upload mode: block until the sensor pushes a frame (up to timeout_ms_)
    // The sensor switches to this mode automatically after 20s with no commands
    std::optional<RadonData> read_active();

private:
    static constexpr size_t FRAME_LEN = 9;

    // Read exactly `len` bytes, respecting timeout
    bool read_exact(uint8_t* buf, size_t len);

    // Sync to start byte (0xFF), then read the rest of the frame
    bool read_frame(uint8_t* buf);

    uint8_t calc_checksum(const uint8_t* buf) const;
    bool    verify_checksum(const uint8_t* buf) const;

    std::string device_;
    int         fd_         = -1;
    int         timeout_ms_ = 5000;
};