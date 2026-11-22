// Pin assignments per chip family. Adjust to your wiring / board.
#pragma once
#include <Arduino.h>

#if defined(ESP8266)
// NodeMCU labels: operator G=D1 R=D2 B=D3, stage G=D5 R=D6 B=D7, WS2812 on D4, onboard LED on D0
constexpr uint8_t PIN_OP_G = 5, PIN_OP_R = 4, PIN_OP_B = 0;
constexpr uint8_t PIN_ST_G = 14, PIN_ST_R = 12, PIN_ST_B = 13;
constexpr uint8_t PIN_WS2812 = 2;
constexpr int PIN_ONBOARD = 16;  // no PWM on GPIO16, driven digitally
constexpr bool ONBOARD_ACTIVE_LOW = true;
#elif defined(ESP32)
constexpr uint8_t PIN_OP_G = 16, PIN_OP_R = 17, PIN_OP_B = 18;
constexpr uint8_t PIN_ST_G = 19, PIN_ST_R = 21, PIN_ST_B = 22;
constexpr uint8_t PIN_WS2812 = 4;
constexpr int PIN_ONBOARD = 2;
constexpr bool ONBOARD_ACTIVE_LOW = false;
#else
#error "Unsupported platform: add pin definitions to src/Board.h"
#endif

constexpr uint8_t MAX_WS2812 = 10;  // per light group
