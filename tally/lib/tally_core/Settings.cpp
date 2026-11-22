#include "Settings.h"

#include <cctype>
#include <cstdlib>

namespace tally {

static std::string trim(const std::string &s) {
    size_t b = 0, e = s.size();
    while (b < e && isspace((unsigned char)s[b])) b++;
    while (e > b && isspace((unsigned char)s[e - 1])) e--;
    return s.substr(b, e - b);
}

static std::string lower(std::string s) {
    for (auto &c : s) c = (char)tolower((unsigned char)c);
    return s;
}

// strict integer parsing: optional sign followed by digits only
static bool parseInt(const std::string &s, int &out) {
    if (s.empty()) return false;
    size_t i = (s[0] == '-' || s[0] == '+') ? 1 : 0;
    if (i == s.size()) return false;
    for (size_t j = i; j < s.size(); j++)
        if (!isdigit((unsigned char)s[j])) return false;
    long v = strtol(s.c_str(), nullptr, 10);
    if (v > 1000000 || v < -1000000) return false;
    out = (int)v;
    return true;
}

std::string makeHostName(const std::string &name) {
    std::string out;
    bool lastDash = false;
    for (char c : name) {
        if (isalnum((unsigned char)c)) {
            out += c;
            lastDash = false;
        } else if (!lastDash) {
            out += '-';
            lastDash = true;
        }
    }
    size_t b = out.find_first_not_of('-');
    if (b == std::string::npos) return "Tally-";
    size_t e = out.find_last_not_of('-');
    return "Tally-" + out.substr(b, e - b + 1);
}

std::string Settings::hostName() const { return makeHostName(name); }

static bool parseLightType(const std::string &v, LightType &out) {
    std::string l = lower(v);
    if (l == "grb+") out = LightType::CommonAnode;
    else if (l == "grb-") out = LightType::CommonCathode;
    else return false;
    return true;
}

static void parseWs2812(const std::string &key, const std::string &v, int &lights, Ws2812Type &type,
                        std::vector<std::string> &warnings) {
    std::string num = v, typ;
    bool hasType = false;
    size_t idx = v.find(' ');
    if (idx != std::string::npos) {
        num = v.substr(0, idx);
        typ = v.substr(idx + 1);
        hasType = true;
    }
    int n;
    if (parseInt(num, n) && n >= 0 && n <= 10) {
        lights = n;
    } else {
        warnings.push_back(key + " needs to be number between 0 and 10, but got \"" + v + "\"");
    }
    if (hasType) {
        if (typ == "rgb") type = Ws2812Type::RGB;
        else if (typ == "grb") type = Ws2812Type::GRB;
        else warnings.push_back(key + " should have a supported light type, but got \"" + typ + "\"");
    }
}

void parseSettings(const std::string &content, Settings &s, std::vector<std::string> &warnings) {
    size_t pos = 0;
    while (pos <= content.size()) {
        size_t nl = content.find('\n', pos);
        std::string line = content.substr(pos, nl == std::string::npos ? std::string::npos : nl - pos);
        pos = (nl == std::string::npos) ? content.size() + 1 : nl + 1;

        if (line.empty() || line[0] == ';') continue;
        size_t eq = line.find('=');
        if (eq == std::string::npos) continue;
        std::string k = trim(line.substr(0, eq));
        std::string v = trim(line.substr(eq + 1));
        if (k.empty() || v.empty()) continue;

        if (k == "station.ssid") s.staSsid = v;
        else if (k == "station.password") s.staPw = v;
        else if (k == "hub.ip") s.hubIp = v;
        else if (k == "hub.port") {
            int p;
            if (parseInt(v, p) && p > 0 && p < 65536) s.hubPort = p;
            else warnings.push_back("Invalid hub.port \"" + v + "\"");
        } else if (k == "tally.name") s.name = v.substr(0, 26);
        else if (k == "operator.type") {
            if (!parseLightType(v, s.operatorType)) warnings.push_back("Invalid operator.type \"" + v + "\"");
        } else if (k == "stage.type") {
            if (!parseLightType(v, s.stageType)) warnings.push_back("Invalid stage.type \"" + v + "\"");
        } else if (k == "operator.ws2812") parseWs2812(k, v, s.operatorWs2812Lights, s.operatorWs2812Type, warnings);
        else if (k == "stage.ws2812") parseWs2812(k, v, s.stageWs2812Lights, s.stageWs2812Type, warnings);
        else warnings.push_back("Unknown key " + k + " in configuration file. Did you mistype it?");
    }
}

}  // namespace tally
