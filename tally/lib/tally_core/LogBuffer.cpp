#include "LogBuffer.h"

namespace tally {

const char *severityName(Severity s) {
    switch (s) {
        case Severity::Error: return "ERROR";
        case Severity::Warn: return "WARN";
        default: return "INFO";
    }
}

void LogBuffer::add(Severity severity, const std::string &msg) {
    if (buffer_.size() >= maxLength) {
        int idx = -1;
        int lowest = 0;
        for (size_t i = 0; i < buffer_.size(); i++) {
            int other = (int)buffer_[i].severity;
            if (other <= (int)severity && (idx < 0 || other < lowest)) {
                idx = (int)i;
                lowest = other;
            }
        }
        if (idx >= 0) buffer_.erase(buffer_.begin() + idx);
    }
    if (buffer_.size() < maxLength) buffer_.push_back({severity, msg.substr(0, maxStringLength)});
}

LogBuffer::Record LogBuffer::take() {
    Record r = buffer_.front();
    buffer_.pop_front();
    return r;
}

}  // namespace tally
