// Parser for the UDP messages sent by the hub, e.g.
//   "O010/020/030 S040/050/060"             static color
//   "O010/020/030 S040/050/060 0xAA 300"    flashing: 8 step pattern, step duration in ms
#pragma once
#include <stdint.h>
#include <string>

namespace tally {

struct TallyMessage {
    uint8_t opR = 0, opG = 0, opB = 0;
    uint8_t stR = 0, stG = 0, stB = 0;
    bool hasPattern = false;
    bool pattern[8] = {false};  // pattern[0] is the first step (MSB of the hex byte)
    uint16_t stepDuration = 0;  // ms
};

bool parseMessage(const std::string &raw, TallyMessage &out);

}  // namespace tally
