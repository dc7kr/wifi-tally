#pragma once

namespace WifiManager {
void connect();
void loop();
/// true only when associated and an IP was received
bool isConnected();
}
