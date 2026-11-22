#include "TallyLink.h"

#include <Arduino.h>
#include <WiFiUdp.h>

#include "Leds.h"
#include "Log.h"
#include "Protocol.h"
#include "Settings.h"
#include "WifiManager.h"

extern tally::Settings settings;

namespace {
const uint16_t listenPort = 7411;
WiFiUDP udp;
bool listening = false;
bool everReceived = false;
uint32_t timeLastPackage = 0;
uint32_t lastProbe = 0;

void send(const std::string &data) {
    if (!WifiManager::isConnected()) {
        Log::error("Not sending packet, because wifi is not connected.");
        return;
    }
    if (!listening) {
        Log::error("Not sending packet, because UDP is not set up yet.");
        return;
    }
    udp.beginPacket(settings.hubIp.c_str(), settings.hubPort);
    std::string out = data + "\n";
    udp.write((const uint8_t *)out.data(), out.size());
    udp.endPacket();
}

void sendInfo() { send("tally-ho \"" + settings.name + "\""); }

void handlePacket(const std::string &data) {
    everReceived = true;
    timeLastPackage = millis();
    tally::TallyMessage m;
    if (!tally::parseMessage(data, m)) {
        Log::warning("invalid package: " + data);
        return;
    }
    if (m.hasPattern) Leds::showFlash(m.opR, m.opG, m.opB, m.stR, m.stG, m.stB, m.pattern, 8, m.stepDuration);
    else Leds::showStatic(m.opR, m.opG, m.opB, m.stR, m.stG, m.stB);
}
}  // namespace

namespace TallyLink {

void connect() {
    if (!listening) listening = udp.begin(listenPort);
    Log::info("Listening for hub on port " + std::to_string(listenPort));
    Log::info("Contacting hub on " + settings.hubIp + ":" + std::to_string(settings.hubPort));
    sendInfo();
}

bool isReady() { return WifiManager::isConnected() && listening; }
bool isConnected() { return everReceived && millis() - timeLastPackage <= 3000; }

void sendLog(const char *severity, const std::string &msg) {
    send("log \"" + settings.name + "\" " + severity + " \"" + msg + "\"");
}

void loop() {
    if (listening) {
        int size = udp.parsePacket();
        if (size > 0) {
            char buf[64];
            int n = udp.read(buf, sizeof(buf) - 1);
            if (n > 0) handlePacket(std::string(buf, n));
        }
    }

    uint32_t now = millis();
    if (now - lastProbe >= 1000) {
        lastProbe = now;
        if (WifiManager::isConnected()) {
            // lost the connection to the base station?
            if (!isConnected()) Leds::waitForServerConnection();
            // probe, so the hub knows that we are still there
            if (isReady()) sendInfo();
        }
    }
}

}  // namespace TallyLink
