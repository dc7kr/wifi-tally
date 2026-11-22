#include <Arduino.h>

#include "Leds.h"
#include "Log.h"
#include "Settings.h"
#include "Settings_store.h"
#include "TallyLink.h"
#include "WifiManager.h"

tally::Settings settings;

static std::string chipId() {
    char buf[12];
#if defined(ESP8266)
    snprintf(buf, sizeof(buf), "%x", ESP.getChipId());
#else
    snprintf(buf, sizeof(buf), "%x", (unsigned)(ESP.getEfuseMac() >> 24));
#endif
    return buf;
}

static std::string resetReason() {
#if defined(ESP8266)
    return ESP.getResetReason().c_str();
#else
    return "reset reason " + std::to_string((int)esp_reset_reason());  // see esp_reset_reason_t
#endif
}

void setup() {
    Serial.begin(115200);
    Serial.println();

    settings.name = chipId();
    std::vector<std::string> warnings;
    bool fileFound = SettingsStore::load(settings, warnings);

    Leds::begin(settings);

    Log::info("booted because of " + resetReason());
    if (!fileFound) Log::warning("Configuration file /tally-settings.ini does not exist. Using defaults.");
    for (const auto &w : warnings) Log::warning(w);

    if (!settings.isValid()) {
        Leds::invalidSettingsFile();
    } else {
        Leds::initial();
        WifiManager::connect();
    }
}

void loop() {
    if (!settings.isValid()) {
        Leds::loop();
        return;
    }
    WifiManager::loop();
    TallyLink::loop();
    Leds::loop();
    Log::loop();
}
