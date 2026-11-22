#pragma once
#include <string>

namespace TallyLink {
/// starts listening for the hub and announces this tally. Needs WiFi to be connected.
void connect();
void loop();
/// WiFi is up and the UDP socket is listening
bool isReady();
/// a package from the hub was received within the last 3 seconds
bool isConnected();
void sendLog(const char *severity, const std::string &msg);
}  // namespace TallyLink
