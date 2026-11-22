#pragma once
#include <string>

#include "LogBuffer.h"

namespace Log {
void info(const std::string &msg);
void warning(const std::string &msg);
void error(const std::string &msg);
/// call from loop(): forwards buffered logs to the hub once it is connected
void loop();
}  // namespace Log
