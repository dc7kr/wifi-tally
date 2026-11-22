#pragma once
#include <stdint.h>

#include "Settings.h"

namespace Leds {
void begin(const tally::Settings &settings);
/// call from loop(): advances flash patterns
void loop();

void initial();
void waitForWifiConnection();
void waitForWifiIp();
void waitForServerConnection();
void invalidSettingsFile();

void showStatic(uint8_t opR, uint8_t opG, uint8_t opB, uint8_t stR, uint8_t stG, uint8_t stB);
void showFlash(uint8_t opR, uint8_t opG, uint8_t opB, uint8_t stR, uint8_t stG, uint8_t stB,
               const bool *pattern, uint8_t patternLength, uint16_t stepDuration);
}  // namespace Leds
