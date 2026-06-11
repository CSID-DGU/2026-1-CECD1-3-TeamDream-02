# Cross-compilation: set CROSS_COMPILE and ARCH_FLAGS from the environment or
# pass them on the command line.  deploy.sh sets these automatically based on
# the Pi's reported architecture.
#
#   Native (default):
#     make
#
#   RPi Zero (ARMv6, 32-bit OS):
#     make CROSS_COMPILE=arm-linux-gnueabihf- \
#          ARCH_FLAGS="-march=armv6 -mfpu=vfp -mfloat-abi=hard"
#
#   RPi Zero 2 / 2W (ARMv8, 32-bit OS):
#     make CROSS_COMPILE=arm-linux-gnueabihf- \
#          ARCH_FLAGS="-march=armv8-a -mfpu=neon-fp-armv8 -mfloat-abi=hard"
#
#   RPi Zero 2 / 2W (ARMv8, 64-bit OS):
#     make CROSS_COMPILE=aarch64-linux-gnu-

CROSS_COMPILE ?=
ARCH_FLAGS    ?=

CXX      := $(CROSS_COMPILE)g++
CXXFLAGS := -std=c++17 -Wall -Wextra -O2 $(ARCH_FLAGS)
TARGET   := radon_sensor
SRCS     := src/main.cpp src/radon_sensor.cpp
OBJS     := $(SRCS:.cpp=.o)

.PHONY: all clean install uninstall

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(CXXFLAGS) -o $@ $^

%.o: %.cpp
	$(CXX) $(CXXFLAGS) -c -o $@ $<

src/radon_sensor.o: src/radon_sensor.cpp src/radon_sensor.hpp
src/main.o:         src/main.cpp         src/radon_sensor.hpp

clean:
	rm -f $(OBJS) $(TARGET)

install: $(TARGET)
	install -m 755 $(TARGET) /usr/local/bin/$(TARGET)
	id -u radon &>/dev/null || useradd -r -s /sbin/nologin radon
	usermod -aG dialout radon
	cp radon-sensor.service /etc/systemd/system/
	systemctl daemon-reload
	systemctl enable --now radon-sensor

uninstall:
	systemctl disable --now radon-sensor || true
	rm -f /etc/systemd/system/radon-sensor.service
	rm -f /usr/local/bin/$(TARGET)
	systemctl daemon-reload