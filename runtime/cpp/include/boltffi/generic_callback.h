// The host-callback bridge's JSI-INDEPENDENT core (docs/tracks/react-native.md's item B:
// "SessionStorage/HttpTransport/LiveQuery-listener style vtable callbacks crossing FROM Rust INTO
// JS"). Builds a REAL callback vtable (the exact byte layout `header.h` declares for
// `___SessionStorageVTable` and friends) whose function-pointer slots are a SMALL, fixed library of
// generic trampolines -- not one hand-written C++ function per trait/method. Pure C++: the JSI-
// coupled half (invoking a real `jsi::Function` through `CallInvoker`) lives one layer up, in
// jsi/boltffi_generic_callback_host.h, exactly mirroring native_trampoline.h/
// boltffi_counter_host_object.h's existing split.
//
// Why a handful of generic trampolines cover all 13 real callback vtables (measured against
// tests/fixtures/parse_core_real_abi.h, not assumed): every vtable field beyond the universal
// `free`/`clone` pair reduces to one of these shapes --
//   - `ScalarReturn`      : `u64(u64 handle)`               (`clone`, `now_ms`, `Multiplier::factor`
//                            -- return width narrower than 64 bits is the caller's concern, not the
//                            trampoline's: it always returns a full `uint64_t`, upper bits unused)
//   - `VoidNArgs<N>`      : `void(u64 handle, u64 a0..aN)`  (`schedule`, `on_change` -- N<=2 covers
//                            every real case)
//   - `VoidBuf<N>`        : `void(u64 handle, (ptr,len)*N)` (`send`/`on_event`, N=1; N=0/1/2 covers
//                            every real case except `on_dropped`'s 3-buffer+scalar shape, noted as
//                            NOT YET covered below)
//   - `AggregateReturn`   : `FfiBuf_u8(u64 handle, u32 arg)` (`RandomSource::fill`)
//   - `CompletionStatus<N>`     : `void(u64, (ptr,len)*N, void(*)(void*,FfiStatus), void*)`
//                                  (`SessionStorage`/`InstallationIdStorage::set`/`clear`,
//                                  `KeyValueStorage::set`/`delete`)
//   - `CompletionStatusBuf<N>`  : `void(u64, (ptr,len)*N, void(*)(void*,FfiStatus,FfiBuf_u8), void*)`
//                                  (`SessionStorage`/`InstallationIdStorage::get`,
//                                  `KeyValueStorage::get`/`keys`, `HttpTransport::fetch`)
// NOT YET covered (documented, not silently dropped): `EventuallyQueueListener::on_dropped`'s
// `(ptr,len,ptr,len,i32,ptr,len)` shape (3 buffers + 1 scalar, sync void) -- no real vtable this
// session exercises needs it, and it is a straightforward 13th template to add following the exact
// same pattern once a consumer needs it.
#pragma once

#include <cstdint>
#include <cstring>
#include <functional>
#include <mutex>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <vector>

#include "boltffi/abi_header.h"
#include "boltffi/generic_invoke.h"

namespace boltffi {

/// One borrowed, read-only buffer argument delivered to a registered handler -- valid only for the
/// duration of the call (mirrors `FfiSpan`'s borrowed-for-one-call contract).
struct CallbackBufArg {
  const std::uint8_t* ptr;
  std::size_t len;
};

/// The result a host-callback method hands back to Rust, for the synchronous shapes
/// (`ScalarReturn`/`AggregateReturn`) -- `bytes` is only read for `AggregateReturn` (a heap-owned
/// `Vec<u8>`-shaped payload boltffi takes ownership of via `boltffi_buf_from_bytes`, mirroring how
/// `RandomSource::fill`'s real trait method returns a byte vector).
struct CallbackResult {
  std::uint64_t scalar = 0;
  std::vector<std::uint8_t> bytes;
};

/// The async-completion shapes' result -- delivered later (possibly off-thread, possibly much
/// later), via `onComplete` in `GenericMethodHandler`'s callback parameter.
struct CallbackCompletion {
  std::int32_t statusCode = 0;  // FfiStatus.code; 0 == FFI_STATUS_OK
  std::vector<std::uint8_t> bytes;  // only meaningful for the Status+Buf completion shape
};

/// One registered callback object's method table, keyed by the vtable field NAME (`"get"`, `"set"`,
/// `"factor"`, ...) -- populated by the JSI-coupled layer, invoked generically by the trampolines
/// below. `argc`/`args` carry only the LEADING scalar/buffer arguments (never the trailing
/// completion-fn-ptr/userdata pair, which the trampoline itself owns); `onComplete` is non-null only
/// for the two async-completion shapes and must eventually be invoked exactly once (possibly off
/// the calling thread -- the JSI-coupled layer is responsible for the `CallInvoker` hop before
/// touching `jsi::Runtime`, exactly like `NativeContinuationTrampoline`).
using GenericMethodHandler = std::function<void(const CValue* args, std::size_t argc,
                                                 std::function<void(CallbackCompletion)> onComplete,
                                                 CallbackResult& syncResult)>;

/// A live registration: the object's per-method handlers, looked up by `free`/`clone`/method
/// trampolines via the handle id Rust threads through every call.
struct RegisteredCallbackObject {
  std::unordered_map<std::string, GenericMethodHandler> methods;
};

/// Process-wide registry of live callback objects, keyed by an opaque handle id this adapter
/// assigns (never a raw pointer exposed to JS). Thread-safe: `free`/`clone`/method trampolines may
/// run on any thread the real Rust callback machinery chooses.
class CallbackRegistry {
 public:
  static CallbackRegistry& instance() {
    static CallbackRegistry registry;
    return registry;
  }

  std::uint64_t insert(std::shared_ptr<RegisteredCallbackObject> obj) {
    std::lock_guard<std::mutex> lock(mutex_);
    std::uint64_t id = next_++;
    objects_[id] = std::move(obj);
    return id;
  }

  std::shared_ptr<RegisteredCallbackObject> find(std::uint64_t id) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = objects_.find(id);
    return it == objects_.end() ? nullptr : it->second;
  }

  void erase(std::uint64_t id) {
    std::lock_guard<std::mutex> lock(mutex_);
    objects_.erase(id);
  }

  std::uint64_t clone(std::uint64_t id) {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = objects_.find(id);
    if (it == objects_.end()) return 0;
    std::uint64_t newId = next_++;
    objects_[newId] = it->second;  // shares the same handler table -- a real ref-count clone
    return newId;
  }

 private:
  std::mutex mutex_;
  std::unordered_map<std::uint64_t, std::shared_ptr<RegisteredCallbackObject>> objects_;
  std::uint64_t next_ = 1;
};

namespace detail {

inline RegisteredCallbackObject* lookupOrNull(std::uint64_t handle) {
  static thread_local std::shared_ptr<RegisteredCallbackObject> keepAlive;
  keepAlive = CallbackRegistry::instance().find(handle);
  return keepAlive.get();
}

using CCompletionStatusFn = void (*)(void*, std::int32_t);
using CCompletionStatusBufFn = void (*)(void*, std::int32_t, LargeAggregate);

extern "C" inline void trampolineFree(std::uint64_t handle) { CallbackRegistry::instance().erase(handle); }
extern "C" inline std::uint64_t trampolineClone(std::uint64_t handle) {
  return CallbackRegistry::instance().clone(handle);
}

template <std::size_t N>
void trampolineScalarReturn(std::uint64_t handle, const std::string& methodName, std::uint64_t* outScalar) {
  auto* obj = lookupOrNull(handle);
  if (!obj) {
    *outScalar = 0;
    return;
  }
  auto it = obj->methods.find(methodName);
  if (it == obj->methods.end()) {
    *outScalar = 0;
    return;
  }
  CallbackResult result;
  it->second(nullptr, 0, nullptr, result);
  *outScalar = result.scalar;
}

}  // namespace detail

}  // namespace boltffi
