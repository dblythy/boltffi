// Stage-3 acceptance evidence (docs/tracks/react-native.md): compiles and links
// `BoltFFICounterHostObject` -- a real `facebook::jsi::HostObject` subclass -- against the REAL
// `facebook::jsi` library (built from Meta's own jsi.cpp/jsilib-posix.cpp, see CMakeLists.txt),
// then drives its JSI-INDEPENDENT core methods (addSync/delayedAddAsync/setMultiplier/scaledSync)
// against the REAL compiled rn_poc fixture dylib -- proving the dlsym resolution, the
// continuation trampoline, and the callback vtable all genuinely interoperate with the actual
// Rust ABI, not just a synthetic C++ fake.
//
// What this does NOT prove (the honest stage-4 remainder, see runtime/cpp/README.md): the
// `get()`/`getPropertyNames()` JSI wrapping itself is only compiled+linked here, never invoked --
// that requires a live `jsi::Runtime` (Hermes or JSC), which this stage deliberately does not
// vendor.
#include <dlfcn.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdlib>
#include <deque>
#include <functional>
#include <mutex>
#include <thread>

#include "boltffi_counter_host_object.h"

namespace {

// A minimal `CallInvoker` fake: hops onto a dedicated background thread (standing in for RN's
// JS thread) via a simple work queue -- NOT synchronous-in-place, so this genuinely exercises
// the "always hop, never assume the calling thread" discipline the trampoline requires, without
// needing a live jsi::Runtime.
class QueueCallInvoker : public facebook::react::CallInvoker {
 public:
  QueueCallInvoker() : worker_([this] { runLoop(); }) {}

  ~QueueCallInvoker() override {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      stop_ = true;
    }
    cv_.notify_all();
    worker_.join();
  }

  void invokeAsync(facebook::react::CallFunc&& func) noexcept override {
    // Not exercised by `NativeContinuationTrampoline`'s hop (it only ever calls the
    // `std::function<void()>` overload below, overridden directly to avoid needing a live
    // `jsi::Runtime&` just to satisfy this overload's signature -- see that override's comment).
    (void)func;
  }

  void invokeSync(facebook::react::CallFunc&& func) override {
    // Not exercised by this linkcheck.
    (void)func;
  }

  // Overriding this directly (rather than relying on `CallInvoker`'s default implementation,
  // which wraps into the `CallFunc` overload above and therefore needs a real `jsi::Runtime&`)
  // is what lets this linkcheck drive the real trampoline+CallInvoker hop end-to-end without
  // vendoring a JS engine: `NativeContinuationTrampoline`'s `ThreadHop` only ever calls this
  // exact overload (see native_trampoline.h -- `ThreadHop` is deliberately Runtime-agnostic).
  void invokeAsync(std::function<void()>&& func) noexcept override { push(std::move(func)); }

  void push(std::function<void()> fn) {
    {
      std::lock_guard<std::mutex> lock(mutex_);
      queue_.push_back(std::move(fn));
    }
    cv_.notify_all();
  }

 private:
  void runLoop() {
    while (true) {
      std::function<void()> fn;
      {
        std::unique_lock<std::mutex> lock(mutex_);
        cv_.wait(lock, [this] { return stop_ || !queue_.empty(); });
        if (stop_ && queue_.empty()) return;
        fn = std::move(queue_.front());
        queue_.pop_front();
      }
      fn();
    }
  }

  std::mutex mutex_;
  std::condition_variable cv_;
  std::deque<std::function<void()>> queue_;
  bool stop_ = false;
  std::thread worker_;
};

int check(bool cond, const char* what) {
  std::printf("%s %s\n", cond ? "[ OK ]" : "[FAIL]", what);
  return cond ? 0 : 1;
}

}  // namespace

int main(int argc, char** argv) {
  const char* dylibPath =
      argc > 1 ? argv[1]
                : "runtime/typescript/test/fixtures/rn_poc/target/release/librn_poc.dylib";

  void* dylib = dlopen(dylibPath, RTLD_NOW | RTLD_LOCAL);
  if (!dylib) {
    std::fprintf(stderr, "dlopen failed for %s: %s\n", dylibPath, dlerror());
    return 1;
  }

  int failures = 0;
  auto invoker = std::make_shared<QueueCallInvoker>();

  boltffi::BoltFFICounterHostObject host(dylib, invoker, /*start=*/10);

  failures += check(host.addSync(5) == 15, "addSync(5) == 15 after starting at 10");
  failures += check(host.addSync(2) == 17, "addSync(2) == 17");

  std::atomic<bool> asyncDone{false};
  std::atomic<int32_t> asyncResult{0};
  std::atomic<int32_t> asyncStatus{-1};
  host.delayedAddAsync(23, [&](std::int32_t result, std::int32_t status) {
    asyncResult = result;
    asyncStatus = status;
    asyncDone = true;
  });

  for (int i = 0; i < 400 && !asyncDone.load(); ++i) {
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
  failures += check(asyncDone.load(), "delayedAddAsync completed within timeout");
  failures += check(asyncStatus.load() == 0, "delayedAddAsync status == Ok");
  failures += check(asyncResult.load() == 40, "delayedAddAsync(23) == 40 (17 + 23)");

  host.setMultiplier([] { return 4; });
  failures += check(host.scaledSync() == 160, "scaledSync() == 160 (40 * 4)");

  std::printf("%d failure(s)\n", failures);
  dlclose(dylib);
  return failures == 0 ? 0 : 1;
}
