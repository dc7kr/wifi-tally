// Buffers logs while the hub is not connected yet. Lives in RAM, so keep the limits small.
#pragma once
#include <deque>
#include <string>

namespace tally {

enum class Severity { Info = 0, Warn = 1, Error = 2 };

const char *severityName(Severity s);

class LogBuffer {
   public:
    static constexpr size_t maxLength = 10;
    static constexpr size_t maxStringLength = 80;

    struct Record {
        Severity severity;
        std::string msg;
    };

    /// When full, the oldest log with the lowest severity (not higher than the new one) is dropped.
    /// If there is none, the new log is discarded.
    void add(Severity severity, const std::string &msg);
    bool hasLog() const { return !buffer_.empty(); }
    /// removes and returns the oldest log. Check hasLog() first.
    Record take();

   private:
    std::deque<Record> buffer_;
};

}  // namespace tally
