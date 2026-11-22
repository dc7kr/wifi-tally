#include "WifiManager.h"

#include <Arduino.h>
#if defined(ESP8266)
#include <ESP8266WiFi.h>
#else
#include <WiFi.h>
#endif

#include "Leds.h"
#include "Log.h"
#include "Settings.h"
#include "TallyLink.h"

extern tally::Settings settings;

namespace {
bool connected = false;
bool reconnectPending = false;
uint32_t reconnectAt = 0;
bool lastDisconnectReasonKnown = false;
uint8_t lastDisconnectReason = 0;

void startConnection() {
    Log::info("Connect to WiFi " + settings.staSsid);
    Leds::waitForWifiConnection();
    WiFi.begin(settings.staSsid.c_str(), settings.staPw.c_str());
}

void scheduleReconnect(uint32_t delayMs) {
    reconnectPending = true;
    reconnectAt = millis() + delayMs;
}

#if defined(ESP8266)
WiFiEventHandler disconnectHandler;
#endif
}  // namespace

namespace WifiManager {

void connect() {
    WiFi.persistent(false);
    WiFi.mode(WIFI_STA);
    WiFi.setAutoReconnect(false);
#if defined(ESP8266)
    WiFi.hostname(settings.hostName().c_str());
    WiFi.setOutputPower(20.5);
    disconnectHandler = WiFi.onStationModeDisconnected([](const WiFiEventStationModeDisconnected &e) {
        lastDisconnectReasonKnown = true;
        lastDisconnectReason = e.reason;
    });
#else
    WiFi.setHostname(settings.hostName().c_str());
    WiFi.setTxPower(WIFI_POWER_19_5dBm);
    WiFi.onEvent([](arduino_event_id_t, arduino_event_info_t info) {
        lastDisconnectReasonKnown = true;
        lastDisconnectReason = info.wifi_sta_disconnected.reason;
    }, ARDUINO_EVENT_WIFI_STA_DISCONNECTED);
#endif
    startConnection();
}

bool isConnected() { return connected; }

void loop() {
    bool up = WiFi.status() == WL_CONNECTED;  // associated *and* has an IP

    if (up && !connected) {
        connected = true;
        reconnectPending = false;
        Log::info("Connected to " + settings.staSsid + ". Got IP " + WiFi.localIP().toString().c_str());
        Leds::waitForServerConnection();
        TallyLink::connect();
    } else if (!up && connected) {
        connected = false;
        std::string msg = "Got disconnected from " + settings.staSsid + ".";
        if (lastDisconnectReasonKnown) msg += " Reason " + std::to_string(lastDisconnectReason);
        Log::error(msg);
        Leds::waitForWifiConnection();
        WiFi.disconnect();
        // reason 2 = AUTH_EXPIRE: retry quickly
        scheduleReconnect(lastDisconnectReasonKnown && lastDisconnectReason == 2 ? 200 : 2000);
    } else if (!up && !connected && !reconnectPending && WiFi.status() != WL_DISCONNECTED) {
        // connection attempt failed (wrong password, AP not found, ...)
        Log::error("WiFi connection failed");
        WiFi.disconnect();
        scheduleReconnect(2000);
    }

    if (reconnectPending && (int32_t)(millis() - reconnectAt) >= 0) {
        reconnectPending = false;
        startConnection();
    }
}

}  // namespace WifiManager
