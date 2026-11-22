#include "Settings_store.h"

#include <Arduino.h>
#include <LittleFS.h>

namespace SettingsStore {

static const char *fileName = "/tally-settings.ini";

bool load(tally::Settings &settings, std::vector<std::string> &warnings) {
    if (!LittleFS.begin()) {
        warnings.push_back("Could not mount the file system");
        return false;
    }
    if (!LittleFS.exists(fileName)) return false;
    File f = LittleFS.open(fileName, "r");
    if (!f) {
        warnings.push_back(std::string("Could not open settings file ") + fileName + " to read configuration");
        return false;
    }
    std::string content;
    while (f.available()) content += (char)f.read();
    f.close();
    tally::parseSettings(content, settings, warnings);
    return true;
}

}  // namespace SettingsStore
