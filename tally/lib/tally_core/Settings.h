// Platform independent settings parser. No Arduino dependencies, so it can be unit tested on the host.
#pragma once
#include <string>
#include <vector>

namespace tally {

enum class LightType { CommonAnode, CommonCathode };
enum class Ws2812Type { RGB, GRB };

struct Settings {
    std::string staSsid;
    std::string staPw;
    std::string hubIp;
    int hubPort = 7411;
    std::string name;  // defaults to the chip id, set by the caller
    LightType operatorType = LightType::CommonAnode;
    Ws2812Type operatorWs2812Type = Ws2812Type::GRB;
    int operatorWs2812Lights = 5;
    LightType stageType = LightType::CommonAnode;
    Ws2812Type stageWs2812Type = Ws2812Type::GRB;
    int stageWs2812Lights = 0;

    bool isValid() const { return !staSsid.empty() && !hubIp.empty(); }
    std::string hostName() const;
};

/// Parses the content of an ini file into `settings`. Unknown/invalid lines add a message to `warnings`.
void parseSettings(const std::string &content, Settings &settings, std::vector<std::string> &warnings);

std::string makeHostName(const std::string &name);

}  // namespace tally
