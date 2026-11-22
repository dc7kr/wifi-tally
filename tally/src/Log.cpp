#include "Log.h"

#include <Arduino.h>

#include "TallyLink.h"
#include "WifiManager.h"

using namespace tally;

namespace {
LogBuffer buffer;
uint32_t lastDrain = 0;

void log(Severity severity, const std::string &msg) {
    Serial.printf("%-7s %s\n", (std::string("[") + severityName(severity) + "]").c_str(), msg.c_str());
    if (TallyLink::isReady() && !buffer.hasLog()) {
        TallyLink::sendLog(severityName(severity), msg);
    } else {
        buffer.add(severity, msg);
    }
}
}  // namespace

namespace Log {
void info(const std::string &msg) { log(Severity::Info, msg); }
void warning(const std::string &msg) { log(Severity::Warn, msg); }
void error(const std::string &msg) { log(Severity::Error, msg); }

void loop() {
    uint32_t now = millis();
    if (now - lastDrain < 50) return;
    lastDrain = now;
    if (buffer.hasLog() && TallyLink::isReady() && TallyLink::isConnected() && WifiManager::isConnected()) {
        auto r = buffer.take();
        TallyLink::sendLog(severityName(r.severity), r.msg);
    }
}
}  // namespace Log
