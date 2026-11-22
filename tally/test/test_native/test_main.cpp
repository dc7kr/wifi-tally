#include <unity.h>

#include "LedMath.h"
#include "LogBuffer.h"
#include "Protocol.h"
#include "Settings.h"

using namespace tally;

void setUp() {}
void tearDown() {}

static Settings parse(const std::string &content, std::vector<std::string> *w = nullptr) {
    Settings s;
    s.name = "bc614e";
    std::vector<std::string> warnings;
    parseSettings(content, s, warnings);
    if (w) *w = warnings;
    return s;
}

static const std::string base =
    "station.ssid=MyWifi\nstation.password=topsecret\nhub.ip=10.10.1.1\nhub.port=4242\ntally.name=Doe\n";

// ---- settings ----
void test_settings_all() {
    Settings s = parse(base);
    TEST_ASSERT_EQUAL_STRING("MyWifi", s.staSsid.c_str());
    TEST_ASSERT_EQUAL_STRING("topsecret", s.staPw.c_str());
    TEST_ASSERT_EQUAL_STRING("10.10.1.1", s.hubIp.c_str());
    TEST_ASSERT_EQUAL(4242, s.hubPort);
    TEST_ASSERT_EQUAL_STRING("Doe", s.name.c_str());
    TEST_ASSERT_EQUAL_STRING("Tally-Doe", s.hostName().c_str());
    TEST_ASSERT_TRUE(s.isValid());
}
void test_settings_messy() {
    Settings s = parse("; comment\n\n; station.ssid=Old\nstation.ssid=MyWifi\r\nstation.password = topsecret\r\nhub.ip = 10.10.1.1\nhub.port=4242\ntally.name=Doe");
    TEST_ASSERT_EQUAL_STRING("MyWifi", s.staSsid.c_str());
    TEST_ASSERT_EQUAL_STRING("topsecret", s.staPw.c_str());
    TEST_ASSERT_EQUAL_STRING("10.10.1.1", s.hubIp.c_str());
    TEST_ASSERT_EQUAL(4242, s.hubPort);
}
void test_settings_defaults() {
    Settings s = parse("station.ssid=a\nhub.ip=1.2.3.4\n");
    TEST_ASSERT_EQUAL(7411, s.hubPort);
    TEST_ASSERT_EQUAL_STRING("Tally-bc614e", s.hostName().c_str());
    TEST_ASSERT_TRUE(s.operatorType == LightType::CommonAnode);
    TEST_ASSERT_EQUAL(5, s.operatorWs2812Lights);
    TEST_ASSERT_EQUAL(0, s.stageWs2812Lights);
}
void test_settings_invalid_without_ssid_or_hub() {
    TEST_ASSERT_FALSE(parse("hub.ip=1.2.3.4").isValid());
    TEST_ASSERT_FALSE(parse("station.ssid=a").isValid());
}
void test_settings_name_with_spaces() {
    Settings s = parse("tally.name=John Doe\n");
    TEST_ASSERT_EQUAL_STRING("John Doe", s.name.c_str());
    TEST_ASSERT_EQUAL_STRING("Tally-John-Doe", s.hostName().c_str());
}
void test_settings_weird_name_is_trimmed() {
    Settings s = parse("tally.name= .this   is a -really messed?up name! 8< It's so long it has to be trimmed >8\n");
    TEST_ASSERT_TRUE(s.name.size() <= 26);
    TEST_ASSERT_EQUAL_STRING("Tally-this-is-a-really-messe", s.hostName().c_str());
}
void test_settings_light_type() {
    TEST_ASSERT_TRUE(parse("operator.type=grb-").operatorType == LightType::CommonCathode);
    TEST_ASSERT_TRUE(parse("operator.type=GrB-").operatorType == LightType::CommonCathode);
    TEST_ASSERT_TRUE(parse("stage.type=grb-").stageType == LightType::CommonCathode);
    std::vector<std::string> w;
    Settings s = parse("operator.type=invalid", &w);
    TEST_ASSERT_TRUE(s.operatorType == LightType::CommonAnode);
    TEST_ASSERT_EQUAL(1, (int)w.size());
    TEST_ASSERT_EQUAL_STRING("Invalid operator.type \"invalid\"", w[0].c_str());
}
void test_settings_ws2812() {
    TEST_ASSERT_EQUAL(2, parse("operator.ws2812=2").operatorWs2812Lights);
    Settings s = parse("operator.ws2812=2 rgb\nstage.ws2812=3 rgb");
    TEST_ASSERT_EQUAL(2, s.operatorWs2812Lights);
    TEST_ASSERT_TRUE(s.operatorWs2812Type == Ws2812Type::RGB);
    TEST_ASSERT_EQUAL(3, s.stageWs2812Lights);
    TEST_ASSERT_TRUE(s.stageWs2812Type == Ws2812Type::RGB);
}
void test_settings_ws2812_invalid() {
    for (const char *bad : {"invalid", "-2", "42"}) {
        std::vector<std::string> w;
        Settings s = parse(std::string("operator.ws2812=") + bad, &w);
        TEST_ASSERT_EQUAL(5, s.operatorWs2812Lights);
        TEST_ASSERT_EQUAL(1, (int)w.size());
        TEST_ASSERT_EQUAL_STRING((std::string("operator.ws2812 needs to be number between 0 and 10, but got \"") + bad + "\"").c_str(), w[0].c_str());
    }
    std::vector<std::string> w;
    Settings s = parse("stage.ws2812=2 brg", &w);
    TEST_ASSERT_EQUAL(2, s.stageWs2812Lights);
    TEST_ASSERT_EQUAL_STRING("stage.ws2812 should have a supported light type, but got \"brg\"", w[0].c_str());
}
void test_settings_unknown_key() {
    std::vector<std::string> w;
    parse("foo.bar=1", &w);
    TEST_ASSERT_EQUAL(1, (int)w.size());
}

// ---- protocol ----
void test_protocol_static() {
    TallyMessage m;
    TEST_ASSERT_TRUE(parseMessage("O010/020/030 S040/050/060", m));
    TEST_ASSERT_EQUAL(10, m.opR); TEST_ASSERT_EQUAL(20, m.opG); TEST_ASSERT_EQUAL(30, m.opB);
    TEST_ASSERT_EQUAL(40, m.stR); TEST_ASSERT_EQUAL(50, m.stG); TEST_ASSERT_EQUAL(60, m.stB);
    TEST_ASSERT_FALSE(m.hasPattern);
}
void test_protocol_pattern() {
    TallyMessage m;
    TEST_ASSERT_TRUE(parseMessage("O010/020/030 S040/050/060 0xAA 300\n", m));
    TEST_ASSERT_TRUE(m.hasPattern);
    bool expected[8] = {true, false, true, false, true, false, true, false};
    for (int i = 0; i < 8; i++) TEST_ASSERT_EQUAL(expected[i], m.pattern[i]);
    TEST_ASSERT_EQUAL(300, m.stepDuration);
    TEST_ASSERT_TRUE(parseMessage("O255/255/255 S255/255/255 0x80 300", m));
    TEST_ASSERT_TRUE(m.pattern[0]);
    for (int i = 1; i < 8; i++) TEST_ASSERT_FALSE(m.pattern[i]);
}
void test_protocol_invalid() {
    TallyMessage m;
    TEST_ASSERT_FALSE(parseMessage("this is a very much too long message", m));
    TEST_ASSERT_FALSE(parseMessage("shrt", m));
    TEST_ASSERT_FALSE(parseMessage("X000/000/000 S000/000/000", m));
    TEST_ASSERT_FALSE(parseMessage("O000/000/00a S000/000/000", m));
}

// ---- log buffer ----
void test_log_order_and_empty() {
    LogBuffer b;
    TEST_ASSERT_FALSE(b.hasLog());
    b.add(Severity::Info, "one");
    b.add(Severity::Info, "two");
    TEST_ASSERT_EQUAL_STRING("one", b.take().msg.c_str());
    TEST_ASSERT_EQUAL_STRING("two", b.take().msg.c_str());
    TEST_ASSERT_FALSE(b.hasLog());
}
void test_log_limit_keeps_newest() {
    LogBuffer b;
    for (int i = 1; i <= 20; i++) b.add(Severity::Info, std::to_string(i));
    TEST_ASSERT_EQUAL_STRING("11", b.take().msg.c_str());
}
void test_log_discards_low_severity_first() {
    LogBuffer b;
    for (int i = 0; i < 10; i++) b.add(Severity::Error, "e" + std::to_string(i));
    b.add(Severity::Info, "info");  // dropped: only errors in buffer
    int n = 0;
    while (b.hasLog()) { TEST_ASSERT_TRUE(b.take().msg[0] == 'e'); n++; }
    TEST_ASSERT_EQUAL(10, n);
    for (int i = 0; i < 10; i++) b.add(Severity::Info, "i");
    b.add(Severity::Error, "err");
    int errs = 0, total = 0;
    while (b.hasLog()) { total++; if (b.take().msg == "err") errs++; }
    TEST_ASSERT_EQUAL(10, total);
    TEST_ASSERT_EQUAL(1, errs);
}
void test_log_truncates() {
    LogBuffer b;
    b.add(Severity::Info, std::string(200, 'x'));
    TEST_ASSERT_EQUAL(80, (int)b.take().msg.size());
}

// ---- led math ----
void test_ledmath() {
    TEST_ASSERT_EQUAL(255, pwmValue(0, LightType::CommonAnode));
    TEST_ASSERT_EQUAL(0, pwmValue(255, LightType::CommonAnode));
    TEST_ASSERT_EQUAL(255, pwmValue(255, LightType::CommonCathode));
    TEST_ASSERT_EQUAL(77, dimmed(255));  // ceil(255*0.3)
    TEST_ASSERT_EQUAL(0, dimmed(0));
    TEST_ASSERT_EQUAL(1, dimmed(1));
    TEST_ASSERT_TRUE(onboardLedOn(0, 0, 255));
    TEST_ASSERT_FALSE(onboardLedOn(0, 0, 77));
    TEST_ASSERT_TRUE(onboardLedOn(255, 0, 0));
}

int main() {
    UNITY_BEGIN();
    RUN_TEST(test_settings_all);
    RUN_TEST(test_settings_messy);
    RUN_TEST(test_settings_defaults);
    RUN_TEST(test_settings_invalid_without_ssid_or_hub);
    RUN_TEST(test_settings_name_with_spaces);
    RUN_TEST(test_settings_weird_name_is_trimmed);
    RUN_TEST(test_settings_light_type);
    RUN_TEST(test_settings_ws2812);
    RUN_TEST(test_settings_ws2812_invalid);
    RUN_TEST(test_settings_unknown_key);
    RUN_TEST(test_protocol_static);
    RUN_TEST(test_protocol_pattern);
    RUN_TEST(test_protocol_invalid);
    RUN_TEST(test_log_order_and_empty);
    RUN_TEST(test_log_limit_keeps_newest);
    RUN_TEST(test_log_discards_low_severity_first);
    RUN_TEST(test_log_truncates);
    RUN_TEST(test_ledmath);
    return UNITY_END();
}
