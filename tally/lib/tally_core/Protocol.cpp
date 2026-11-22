#include "Protocol.h"

#include <cctype>

namespace tally {

static bool digits(const std::string &s, size_t pos, size_t len, int base, int &out) {
    int v = 0;
    for (size_t i = pos; i < pos + len; i++) {
        char c = s[i];
        int d;
        if (c >= '0' && c <= '9') d = c - '0';
        else if (base == 16 && c >= 'a' && c <= 'f') d = c - 'a' + 10;
        else if (base == 16 && c >= 'A' && c <= 'F') d = c - 'A' + 10;
        else return false;
        v = v * base + d;
    }
    out = v;
    return true;
}

bool parseMessage(const std::string &raw, TallyMessage &m) {
    size_t b = 0, e = raw.size();
    while (b < e && isspace((unsigned char)raw[b])) b++;
    while (e > b && isspace((unsigned char)raw[e - 1])) e--;
    std::string d = raw.substr(b, e - b);

    size_t len = d.size();
    if (len != 25 && len != 34) return false;
    if (d[0] != 'O' || d[4] != '/' || d[8] != '/' || d.compare(12, 2, " S") != 0 || d[17] != '/' || d[21] != '/')
        return false;

    int v[6];
    const size_t at[6] = {1, 5, 9, 14, 18, 22};
    for (int i = 0; i < 6; i++) {
        if (!digits(d, at[i], 3, 10, v[i]) || v[i] > 255) return false;
    }
    m.opR = v[0]; m.opG = v[1]; m.opB = v[2];
    m.stR = v[3]; m.stG = v[4]; m.stB = v[5];
    m.hasPattern = false;

    if (len == 34) {
        int p, dur;
        if (d.compare(25, 3, " 0x") != 0 || d[30] != ' ') return false;
        if (!digits(d, 28, 2, 16, p) || !digits(d, 31, 3, 10, dur)) return false;
        for (int i = 0; i < 8; i++) m.pattern[i] = (p >> (7 - i)) & 1;
        m.stepDuration = dur;
        m.hasPattern = true;
    }
    return true;
}

}  // namespace tally
