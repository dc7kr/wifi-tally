// Pure helpers for driving the LEDs.
#pragma once
#include <stdint.h>

#include "Settings.h"

namespace tally {

/// PWM value (0-255) for a channel, inverted for common anode lights.
inline uint8_t pwmValue(uint8_t brightness, LightType type) {
    return type == LightType::CommonAnode ? 255 - brightness : brightness;
}

/// brightness used for the "dim" steps of a flash pattern (30%, rounded up)
inline uint8_t dimmed(uint8_t v) { return (uint8_t)((v * 3 + 9) / 10); }

/// The onboard LED is on by default and only turned off while a blue/grey flash pattern is in its dark step.
inline bool onboardLedOn(uint8_t r, uint8_t g, uint8_t b) {
    bool off = (r == 0 && g == 0 && b < 255) || (r == g && g == b && b < 255);
    return !off;
}

}  // namespace tally
