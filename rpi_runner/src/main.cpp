#include "radon_sensor.hpp"

#include <chrono>
#include <csignal>
#include <ctime>
#include <iomanip>
#include <iostream>
#include <thread>

// ── Config ────────────────────────────────────────────────────────────────────
static constexpr char  DEVICE[]       = "/dev/ttyS0";   // RPi Zero: ttyS0 or ttyAMA0
static constexpr int   POLL_INTERVAL  = 60;             // seconds between Q&A queries
static constexpr int   PREHEAT_SEC    = 180;            // datasheet: 3 min preheat
static constexpr int   TIMEOUT_MS     = 5000;

// ── Signal handling ───────────────────────────────────────────────────────────
static volatile sig_atomic_t g_running = 1;

static void sig_handler(int) { g_running = 0; }

// ── Helpers ───────────────────────────────────────────────────────────────────
static std::string timestamp() {
    auto now  = std::chrono::system_clock::now();
    auto tt   = std::chrono::system_clock::to_time_t(now);
    char buf[32];
    std::strftime(buf, sizeof(buf), "%Y-%m-%dT%H:%M:%S", std::localtime(&tt));
    return buf;
}

static void log(const char* level, const char* msg) {
    // journald picks this up via stdout; prefix with level for sd_journal compat
    std::cout << timestamp() << " [" << level << "] " << msg << std::endl;
}

// ── Main ──────────────────────────────────────────────────────────────────────
int main() {
    std::signal(SIGINT,  sig_handler);
    std::signal(SIGTERM, sig_handler);

    RadonSensor sensor(DEVICE, TIMEOUT_MS);

    log("INFO", "radon-sensor starting");

    if (!sensor.open()) {
        std::cerr << "ERROR: cannot open " << DEVICE << std::endl;
        return 1;
    }

    log("INFO", "UART open; waiting for preheat (3 min)…");
    for (int i = 0; i < PREHEAT_SEC && g_running; ++i)
        std::this_thread::sleep_for(std::chrono::seconds(1));

    log("INFO", "preheat done; entering poll loop");

    while (g_running) {
        auto result = sensor.query();

        if (result) {
            // Output in a structured, grep-friendly format
            // journald / prometheus text exporter can parse this
            std::cout << timestamp()
                      << " radon_4h_avg_bq_m3="  << result->avg_4h
                      << " radon_24h_avg_bq_m3=" << result->avg_24h
                      << std::endl;
        } else {
            log("WARN", "query failed or checksum error — retrying next cycle");
        }

        // Sleep in 1-second increments so SIGTERM wakes us promptly
        for (int i = 0; i < POLL_INTERVAL && g_running; ++i)
            std::this_thread::sleep_for(std::chrono::seconds(1));
    }

    log("INFO", "shutting down");
    sensor.close();
    return 0;
}