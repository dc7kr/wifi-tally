#include "Leds.h"

#include <Adafruit_NeoPixel.h>

#include "Board.h"
#include "LedMath.h"

using namespace tally;

namespace {

const Settings *cfg = nullptr;
Adafruit_NeoPixel *strip = nullptr;

// ---- PWM abstraction (ESP8266 / ESP32 core 2.x / ESP32 core 3.x) ----
const uint8_t pwmPins[6] = {PIN_OP_R, PIN_OP_G, PIN_OP_B, PIN_ST_R, PIN_ST_G, PIN_ST_B};

void pwmSetup() {
#if defined(ESP8266)
    analogWriteRange(255);
    analogWriteFreq(1000);
#elif defined(ESP_ARDUINO_VERSION_MAJOR) && ESP_ARDUINO_VERSION_MAJOR >= 3
    for (uint8_t pin : pwmPins) ledcAttach(pin, 1000, 8);
#else
    for (uint8_t ch = 0; ch < 6; ch++) {
        ledcSetup(ch, 1000, 8);
        ledcAttachPin(pwmPins[ch], ch);
    }
#endif
}

void pwmWrite(uint8_t idx, uint8_t value) {
#if defined(ESP8266)
    analogWrite(pwmPins[idx], value);
#elif defined(ESP_ARDUINO_VERSION_MAJOR) && ESP_ARDUINO_VERSION_MAJOR >= 3
    ledcWrite(pwmPins[idx], value);
#else
    ledcWrite(idx, value);
#endif
}

void showColor(uint8_t opR, uint8_t opG, uint8_t opB, uint8_t stR, uint8_t stG, uint8_t stB) {
    pwmWrite(0, pwmValue(opR, cfg->operatorType));
    pwmWrite(1, pwmValue(opG, cfg->operatorType));
    pwmWrite(2, pwmValue(opB, cfg->operatorType));
    pwmWrite(3, pwmValue(stR, cfg->stageType));
    pwmWrite(4, pwmValue(stG, cfg->stageType));
    pwmWrite(5, pwmValue(stB, cfg->stageType));

    if (strip) {
        // the strip is configured as GRB; groups wired as RGB need red and green swapped
        auto color = [](uint8_t r, uint8_t g, uint8_t b, Ws2812Type t) {
            return t == Ws2812Type::GRB ? strip->Color(r, g, b) : strip->Color(g, r, b);
        };
        int n = 0;
        for (int i = 0; i < cfg->operatorWs2812Lights; i++, n++)
            strip->setPixelColor(n, color(opR, opG, opB, cfg->operatorWs2812Type));
        for (int i = 0; i < cfg->stageWs2812Lights; i++, n++)
            strip->setPixelColor(n, color(stR, stG, stB, cfg->stageWs2812Type));
        strip->show();
    }

    if (PIN_ONBOARD >= 0) {
        bool on = onboardLedOn(opR, opG, opB);
        digitalWrite(PIN_ONBOARD, (on != ONBOARD_ACTIVE_LOW) ? HIGH : LOW);
    }
}

// ---- flash pattern state ----
struct {
    bool active = false;
    uint8_t col[6];
    bool pattern[12];
    uint8_t length = 0;
    uint8_t idx = 0;
    uint16_t duration = 0;
    uint32_t lastStep = 0;
} flash;

void flashStep() {
    if (flash.pattern[flash.idx]) {
        showColor(flash.col[0], flash.col[1], flash.col[2], flash.col[3], flash.col[4], flash.col[5]);
    } else {
        showColor(dimmed(flash.col[0]), dimmed(flash.col[1]), dimmed(flash.col[2]),
                  dimmed(flash.col[3]), dimmed(flash.col[4]), dimmed(flash.col[5]));
    }
    flash.idx = (flash.idx + 1) % flash.length;
}

const bool B = true, D = false;

}  // namespace

namespace Leds {

void begin(const Settings &settings) {
    cfg = &settings;
    pwmSetup();
    if (PIN_ONBOARD >= 0) pinMode(PIN_ONBOARD, OUTPUT);
    int total = settings.operatorWs2812Lights + settings.stageWs2812Lights;
    if (total > 0) {
        strip = new Adafruit_NeoPixel(total, PIN_WS2812, NEO_GRB + NEO_KHZ800);
        strip->begin();
    }
}

void loop() {
    if (!flash.active) return;
    uint32_t now = millis();
    if (now - flash.lastStep >= flash.duration) {
        flash.lastStep = now;
        flashStep();
    }
}

void showStatic(uint8_t opR, uint8_t opG, uint8_t opB, uint8_t stR, uint8_t stG, uint8_t stB) {
    flash.active = false;
    showColor(opR, opG, opB, stR, stG, stB);
}

void showFlash(uint8_t opR, uint8_t opG, uint8_t opB, uint8_t stR, uint8_t stG, uint8_t stB,
               const bool *pattern, uint8_t length, uint16_t stepDuration) {
    if (length == 0 || length > 12 || stepDuration == 0) return;
    uint8_t col[6] = {opR, opG, opB, stR, stG, stB};
    // the same pattern is already running: restarting it would destroy the visual rhythm
    if (flash.active && flash.length == length && flash.duration == stepDuration &&
        memcmp(flash.col, col, 6) == 0 && memcmp(flash.pattern, pattern, length) == 0)
        return;
    memcpy(flash.col, col, 6);
    memcpy(flash.pattern, pattern, length);
    flash.length = length;
    flash.duration = stepDuration;
    flash.idx = 0;
    flash.active = true;
    flash.lastStep = millis();
    flashStep();
}

void initial() { showStatic(0, 0, 255, 0, 0, 0); }

void waitForWifiConnection() {
    static const bool p[] = {B, D};
    showFlash(0, 0, 255, 0, 0, 0, p, 2, 500);
}
void waitForWifiIp() {
    static const bool p[] = {B, D, B, D};
    showFlash(0, 0, 255, 0, 0, 0, p, 4, 250);
}
void waitForServerConnection() {
    static const bool p[] = {B, D, B, D, D, D};
    showFlash(0, 0, 255, 0, 0, 0, p, 6, 150);
}
void invalidSettingsFile() {
    static const bool p[] = {B, D, B, D, B, D, D, D, D, D, D, D};
    showFlash(0, 0, 255, 0, 0, 0, p, 12, 150);
}

}  // namespace Leds
