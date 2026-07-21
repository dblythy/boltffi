// A deliberately tiny, dependency-free test harness (no gtest/catch2 fetch -- keeps this target
// buildable with nothing beyond a C++17 compiler, matching the fork's own "cheapest honest path"
// instinct for a first stage-3 slice). Each test function returns void and calls BOLTFFI_CHECK;
// `run()` collects failures and reports a pass/fail summary.
#pragma once

#include <cstdio>
#include <functional>
#include <string>
#include <vector>

namespace boltffi_test {

struct Case {
  std::string name;
  std::function<void()> fn;
};

inline std::vector<Case>& registry() {
  static std::vector<Case> cases;
  return cases;
}

struct Registrar {
  Registrar(const char* name, std::function<void()> fn) {
    registry().push_back(Case{name, std::move(fn)});
  }
};

struct AssertionFailure {
  std::string message;
};

inline int runAll() {
  int failed = 0;
  for (auto& c : registry()) {
    std::printf("[ RUN      ] %s\n", c.name.c_str());
    try {
      c.fn();
      std::printf("[       OK ] %s\n", c.name.c_str());
    } catch (const AssertionFailure& e) {
      std::printf("[  FAILED  ] %s: %s\n", c.name.c_str(), e.message.c_str());
      ++failed;
    } catch (const std::exception& e) {
      std::printf("[  FAILED  ] %s: unexpected exception: %s\n", c.name.c_str(), e.what());
      ++failed;
    }
  }
  std::printf("%zu tests, %d failed\n", registry().size(), failed);
  return failed == 0 ? 0 : 1;
}

}  // namespace boltffi_test

#define BOLTFFI_TEST(name)                                                        \
  static void name();                                                             \
  static ::boltffi_test::Registrar name##_registrar(#name, &name);                \
  static void name()

#define BOLTFFI_CHECK(cond)                                                       \
  do {                                                                            \
    if (!(cond)) {                                                                \
      throw ::boltffi_test::AssertionFailure{                                     \
          std::string("CHECK failed: ") + #cond + " at " __FILE__};              \
    }                                                                             \
  } while (0)
