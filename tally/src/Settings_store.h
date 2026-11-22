#pragma once
#include <string>
#include <vector>

#include "Settings.h"

namespace SettingsStore {
/// Loads /tally-settings.ini from LittleFS. Returns false if the file does not exist.
bool load(tally::Settings &settings, std::vector<std::string> &warnings);
}
