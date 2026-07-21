// The host-callback bridge's JSI-INDEPENDENT core (docs/tracks/react-native.md's item B:
// "SessionStorage/HttpTransport/LiveQuery-listener style vtable callbacks crossing FROM Rust INTO
// JS"). Builds a REAL callback vtable (the exact byte layout `header.h` declares for
// `___SessionStorageVTable` and friends) whose function-pointer slots are drawn from a SMALL, fixed
// library of generic trampolines -- not one hand-written C++ function per trait/method. The JSI-
// coupled half (invoking a real `jsi::Function` through `CallInvoker`) lives one layer up, in
// jsi/boltffi_generic_callback_host.h, mirroring native_trampoline.h/
// boltffi_counter_host_object.h's existing split.
//
// Why a handful of generic trampolines cover ALL 13 of the real callback vtables IN FULL (measured
// against tests/fixtures/parse_core_real_abi.h): every vtable field beyond the universal
// `free`/`clone` pair reduces to one of the shapes below -- including, as of this session's finding-4
// fix, the two shapes that used to fall through `classifyVTableField` as `nullopt` (silently
// installing a NULL vtable slot rather than failing loudly): `EventuallyQueueListener::on_dropped`'s
// `(ptr,len,ptr,len,i32,ptr,len)` shape (3 buffers + 1 scalar, sync void -- `VoidBuf2Scalar1Buf1`)
// and `RandomSource::fill`'s `FfiBuf_u8(uint64_t, uint32_t)` shape (scalar-in, LARGE aggregate-out
// -- `ScalarInBufOut`, whose OWN return-type struct exactly matches `FfiBuf_u8`'s real 32-byte
// layout; unlike `generic_invoke.h`'s caller-side `LargeAggregate`, a callback TRAMPOLINE is the
// ABI's CALLEE: the real caller -- Rust -- allocates exactly `sizeof(the real return type)` bytes,
// so writing a wider struct through that pointer would overflow it -- the mirror-image of
// `generic_invoke.h`'s own sret finding, on the other side of the same coin). Any FUTURE vtable
// shape this file genuinely doesn't cover still returns `nullopt` from `classifyVTableField` (never
// a guess) so `buildVTableBytes` can fail loudly rather than install a null slot.
//
// The "SlotId" mechanism: a C vtable field is a plain function pointer with NO capture -- it can't
// close over "which registered object" or "which method name" the way a `std::function` can. Every
// REGISTRATION already carries that (a `handle` id looked up in `CallbackRegistry`), but a single
// C++ function needs to serve MULTIPLE DIFFERENT trait methods that happen to share the same C
// signature (e.g. `Multiplier::factor` and `Clock::now_ms` are both `uint64_t(uint64_t)`) -- so each
// vtable field this adapter builds gets its own compile-time `SlotId` (0..kMaxCallbackSlots-1,
// assigned once, in order, the first time a vtable of a given shape is built), and the GENERIC
// trampoline for that shape looks up `slotMethodName(SlotId)` (a runtime string, set once at slot-
// assignment time) to find which entry in the registered object's method table to call. This is the
// callback-direction mirror of `generic_invoke.h`'s "closed shape space, not per-symbol code."
#pragma once

#include <dlfcn.h>

#include <array>
#include <cstdint>
#include <cstring>
#include <functional>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <vector>

#include "boltffi/abi_header.h"
#include "boltffi/generic_invoke.h"

namespace boltffi {

/// The result a synchronous host-callback method hands back to Rust (`ScalarReturn`/
/// `AggregateReturn` shapes). `bytes` is only read for `AggregateReturn`.
struct CallbackResult {
  std::uint64_t scalar = 0;
  std::vector<std::uint8_t> bytes;
};

/// The async-completion shapes' result, delivered later (possibly off-thread, possibly much later)
/// by calling the `onComplete` parameter `GenericMethodHandler` receives.
struct CallbackCompletion {
  std::int32_t statusCode = 0;          // FfiStatus.code; 0 == FFI_STATUS_OK
  std::vector<std::uint8_t> bytes;      // only meaningful for the Status+Buf completion shape
};

/// `FfiStatus`'s real `FFI_STATUS_CANCELLED` code (`parse_core_real_abi.h:20`,
/// `#define FFI_STATUS_CANCELLED ((FfiStatus){4})`) -- the status every completion trampoline below
/// reports when the registered object/method is missing (teardown raced the call: `free` ran, or
/// the method was never registered) instead of the default-constructed `CallbackCompletion{}`'s
/// `statusCode == 0` (`FFI_STATUS_OK`). Reporting OK for a completion that never actually ran is a
/// silent correctness bug: Rust (and everything built on top of it) reads success and proceeds as
/// if the operation happened.
constexpr std::int32_t kFfiStatusCancelled = 4;

/// One registered callback object's method table entry. `bufArgs` carries the leading `(ptr,len)`
/// buffer arguments (never the trailing completion-fn-ptr/userdata pair, which the trampoline owns);
/// `scalarArg` carries a single leading scalar (only `AggregateReturn`'s shape uses it today).
/// `onComplete` is non-null only for the two async-completion shapes and must eventually be invoked
/// exactly once -- possibly off the calling thread, so the JSI-coupled layer must hop via
/// `CallInvoker` before touching `jsi::Runtime`, exactly like `NativeContinuationTrampoline`.
struct GenericMethodCall {
  std::vector<std::vector<std::uint8_t>> bufArgs;
  std::uint64_t scalarArg = 0;
  bool hasScalarArg = false;
  std::uint64_t scalarArg2 = 0;
  bool hasScalarArg2 = false;
};
using GenericMethodHandler =
    std::function<void(const GenericMethodCall& call, std::function<void(CallbackCompletion)> onComplete,
                        CallbackResult& syncResult)>;

/// A live registration: one JS-backed object's per-method handlers, looked up by `free`/`clone`/
/// method trampolines via the handle id Rust threads through every call.
struct RegisteredCallbackObject {
  std::unordered_map<std::string, GenericMethodHandler> methods;
};

/// Process-wide registry of live callback objects, keyed by an opaque handle id this adapter
/// assigns (never a raw pointer exposed to Rust/JS). Thread-safe: trampolines may run on any thread
/// the real Rust callback machinery chooses.
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

constexpr std::size_t kMaxCallbackSlots = 128;

namespace detail {

inline std::array<std::string, kMaxCallbackSlots>& slotMethodNames() {
  static std::array<std::string, kMaxCallbackSlots> names;
  return names;
}

inline std::size_t allocateSlot(const std::string& methodName) {
  static std::mutex m;
  static std::size_t next = 0;
  std::lock_guard<std::mutex> lock(m);
  if (next >= kMaxCallbackSlots) throw std::runtime_error("boltffi::generic_callback: out of slots");
  std::size_t slot = next++;
  slotMethodNames()[slot] = methodName;
  return slot;
}

inline std::unordered_map<std::string, std::size_t>& slotCache() {
  static std::unordered_map<std::string, std::size_t> cache;
  return cache;
}

/// Returns the SlotId for `methodName` under `shapeTag` (the calling `CallbackShape`'s own integer
/// value, passed as `int` so this file doesn't need `CallbackShape`'s definition, declared later),
/// allocating a fresh one via `allocateSlot` only the FIRST time this exact (shape, methodName)
/// pair is seen -- every later call for the SAME pair reuses it. Without this, registering the
/// SAME trait/method more than once (multiple `ParseClient` instances installing the same
/// `SessionStorage`-shaped trait, or a hot reload re-registering) would burn a fresh slot from the
/// fixed `kMaxCallbackSlots` budget on every call, exhausting it under nothing more exotic than
/// ordinary repeated use (finding 6) -- a SlotId is process-wide metadata identifying "the
/// trampoline for method X of shape Y," not a per-registration resource.
inline std::size_t allocateOrReuseSlot(int shapeTag, const std::string& methodName) {
  static std::mutex m;
  std::lock_guard<std::mutex> lock(m);
  std::string key = std::to_string(shapeTag) + "|" + methodName;
  auto& cache = slotCache();
  auto it = cache.find(key);
  if (it != cache.end()) return it->second;
  std::size_t slot = allocateSlot(methodName);
  cache.emplace(std::move(key), slot);
  return slot;
}

inline std::shared_ptr<RegisteredCallbackObject> lookup(std::uint64_t handle) {
  return CallbackRegistry::instance().find(handle);
}

// ---- the universal free/clone pair (no SlotId needed -- one instance covers every vtable) ----

extern "C" inline void genericFree(std::uint64_t handle) { CallbackRegistry::instance().erase(handle); }
extern "C" inline std::uint64_t genericClone(std::uint64_t handle) {
  return CallbackRegistry::instance().clone(handle);
}

// ---- ScalarReturn: uint64_t(uint64_t handle) -- `clone`-shaped methods (`factor`, `now_ms`) ----

template <std::size_t SlotId>
std::uint64_t trampolineScalarReturn(std::uint64_t handle) {
  auto obj = lookup(handle);
  if (!obj) return 0;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return 0;
  CallbackResult result;
  GenericMethodCall call;
  it->second(call, nullptr, result);
  return result.scalar;
}

// ---- VoidArgs0: void(uint64_t handle) -- a NAMED void method, e.g. `open_socket`/`close_socket`
// (same C signature as `free`, but must call a named method rather than deregistering).

template <std::size_t SlotId>
void trampolineVoidArgs0(std::uint64_t handle) {
  auto obj = lookup(handle);
  if (!obj) return;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return;
  CallbackResult result;
  GenericMethodCall call;
  it->second(call, nullptr, result);
}

// ---- VoidScalar<N>: void(uint64_t handle, uint64_t * N) -- `on_change` (N=1), `schedule` (N=2).
// Every leading arg is carried as a raw 64-bit bit pattern in `scalarArgs`, matching
// `generic_invoke.h`'s own "every non-float register-class value is a uint64_t" discipline --
// narrower real types (`int32_t`, `___NetworkState`) are the caller-side JS/JSI layer's concern.

template <std::size_t SlotId>
void trampolineVoidScalar1(std::uint64_t handle, std::uint64_t a0) {
  auto obj = lookup(handle);
  if (!obj) return;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return;
  CallbackResult result;
  GenericMethodCall call;
  call.scalarArg = a0;
  call.hasScalarArg = true;
  it->second(call, nullptr, result);
}

template <std::size_t SlotId>
void trampolineVoidScalar2(std::uint64_t handle, std::uint64_t a0, std::uint64_t a1) {
  auto obj = lookup(handle);
  if (!obj) return;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return;
  CallbackResult result;
  GenericMethodCall call;
  call.scalarArg = a0;
  call.hasScalarArg = true;
  call.scalarArg2 = a1;
  call.hasScalarArg2 = true;
  it->second(call, nullptr, result);
}

// ---- VoidBuf<N>: void(uint64_t handle, (const uint8_t*, uintptr_t) * N) -- `send`/`on_event` ----

template <std::size_t SlotId>
void trampolineVoidBuf1(std::uint64_t handle, const std::uint8_t* ptr, std::uintptr_t len) {
  auto obj = lookup(handle);
  if (!obj) return;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return;
  CallbackResult result;
  GenericMethodCall call;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr, ptr + len));
  it->second(call, nullptr, result);
}

// ---- VoidBuf2Scalar1Buf1: void(handle, (ptr,len), (ptr,len), i32, (ptr,len)) -- the real header's
// `EventuallyQueueListener::on_dropped` shape (3 buffers + 1 scalar, sync void, no completion) --
// covering it closes finding 4: before this fix, `classifyVTableField` returned `nullopt` for this
// field and `buildVTableBytes` installed a NULL vtable slot, so Rust invoking `on_dropped` on a
// dropped eventually-queue entry called through a null function pointer.

template <std::size_t SlotId>
void trampolineVoidBuf2Scalar1Buf1(std::uint64_t handle, const std::uint8_t* ptr0, std::uintptr_t len0,
                                    const std::uint8_t* ptr1, std::uintptr_t len1, std::int32_t scalar,
                                    const std::uint8_t* ptr2, std::uintptr_t len2) {
  auto obj = lookup(handle);
  if (!obj) return;
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return;
  CallbackResult result;
  GenericMethodCall call;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr0, ptr0 + len0));
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr1, ptr1 + len1));
  call.scalarArg = static_cast<std::uint64_t>(scalar);
  call.hasScalarArg = true;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr2, ptr2 + len2));
  it->second(call, nullptr, result);
}

// ---- CompletionStatus<N>: void(handle, (ptr,len)*N, void(*)(void*,int32_t), void*) ----
// (`SessionStorage`/`InstallationIdStorage::set`/`clear`, `KeyValueStorage::set`/`delete`)

using CompletionStatusFn = void (*)(void*, std::int32_t);

template <std::size_t SlotId>
void trampolineCompletionStatus0(std::uint64_t handle, CompletionStatusFn complete, void* userdata) {
  auto obj = lookup(handle);
  auto onComplete = [complete, userdata](CallbackCompletion c) { complete(userdata, c.statusCode); };
  if (!obj) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  CallbackResult result;
  GenericMethodCall call;
  it->second(call, onComplete, result);
}

template <std::size_t SlotId>
void trampolineCompletionStatus1(std::uint64_t handle, const std::uint8_t* ptr, std::uintptr_t len,
                                  CompletionStatusFn complete, void* userdata) {
  auto obj = lookup(handle);
  auto onComplete = [complete, userdata](CallbackCompletion c) { complete(userdata, c.statusCode); };
  if (!obj) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  CallbackResult result;
  GenericMethodCall call;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr, ptr + len));
  it->second(call, onComplete, result);
}

// `KeyValueStorage::set`'s real shape: two buffers in (key, value), status-only completion.
template <std::size_t SlotId>
void trampolineCompletionStatus2(std::uint64_t handle, const std::uint8_t* ptr0, std::uintptr_t len0,
                                  const std::uint8_t* ptr1, std::uintptr_t len1, CompletionStatusFn complete,
                                  void* userdata) {
  auto obj = lookup(handle);
  auto onComplete = [complete, userdata](CallbackCompletion c) { complete(userdata, c.statusCode); };
  if (!obj) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  CallbackResult result;
  GenericMethodCall call;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr0, ptr0 + len0));
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr1, ptr1 + len1));
  it->second(call, onComplete, result);
}

// ---- CompletionStatusBuf<N>: void(handle, (ptr,len)*N, void(*)(void*,int32_t,LargeAggregate), void*) ----
// (`SessionStorage`/`InstallationIdStorage::get`, `KeyValueStorage::get`/`keys`,
// `HttpTransport::fetch`) -- the completion function itself takes the >16-byte `FfiBuf_u8` result
// BY VALUE as its third parameter; per generic_invoke.h's own finding, a large by-value PARAMETER
// (not just a return) must be declared with its REAL aggregate C++ type so the compiler emits the
// ABI-correct passing convention (AAPCS64/x86-64 SysV both pass a >16 byte by-value parameter via a
// caller-made temporary + pointer -- the same "let the compiler do it" discipline as `LargeAggregate`
// returns, just on the calling side instead of the returning side this time).

using CompletionStatusBufFn = void (*)(void*, std::int32_t, LargeAggregate);

/// The completion's `FfiBuf_u8` parameter is a real Rust-OWNED allocation (ptr/len/cap/align), not
/// the raw content bytes inlined -- found the hard way (a first version that memcpy'd the payload
/// bytes directly into the by-value parameter crashed inside Rust's allocator on a garbage `len`
/// the very first time this was driven against a real crate, see the design doc's own note on this
/// file). `boltffi_buf_from_bytes` (exported by every crate this adapter links, same as
/// `boltffi_free_buf`) makes the real allocation; this copies ITS four fields into the leading 32
/// bytes of a `LargeAggregate`, which is all `FfiBuf_u8`'s by-value size needs.
inline LargeAggregate makeFfiBufFromBytes(const std::vector<std::uint8_t>& bytes) {
  static void* bufFromBytesFn = dlsym(RTLD_DEFAULT, "boltffi_buf_from_bytes");
  if (!bufFromBytesFn) throw std::runtime_error("boltffi::generic_callback: boltffi_buf_from_bytes not found");
  CValue args[2] = {CValue::ofPtr(bytes.data()), CValue::ofU64(bytes.size())};
  LargeAggregate agg{};
  invokeGenericSret(bufFromBytesFn, args, 2, &agg, 32);
  return agg;
}

// ---- ScalarInBufOut: FfiBuf_u8(uint64_t, uint32_t) -- the real header's `RandomSource::fill`
// shape (scalar-in, LARGE aggregate-out, synchronous). Covering it closes the other half of
// finding 4. `FfiBufReturn` mirrors `FfiBuf_u8`'s real 32-byte layout EXACTLY: unlike
// `generic_invoke.h`'s caller-side `LargeAggregate` (a padded, oversized scratch type a CALLER
// decodes from), a callback TRAMPOLINE is the ABI's CALLEE here -- the real caller (Rust) allocates
// exactly `sizeof(FfiBuf_u8)` bytes for the hidden sret pointer, so returning anything wider (e.g.
// reusing `LargeAggregate`'s 64-byte scratch type as the declared C++ return type) would have the
// compiler write past the space Rust actually reserved. Declaring the REAL, exactly-sized struct as
// this function's return type lets the compiler generate the correct callee-side sret convention
// for whichever real aggregate it is on the current target ABI.
struct FfiBufReturn {
  std::uint8_t* ptr;
  std::uintptr_t len;
  std::uintptr_t cap;
  std::uintptr_t align;
};

template <std::size_t SlotId>
FfiBufReturn trampolineScalarInBufOut(std::uint64_t handle, std::uint32_t n) {
  auto emptyReturn = []() { return FfiBufReturn{nullptr, 0, 0, 0}; };  // FfiBuf::empty() shape --
                                                                        // a no-op to free, matching
                                                                        // ffi_buf.h's own convention.
  auto obj = lookup(handle);
  if (!obj) return emptyReturn();
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) return emptyReturn();
  CallbackResult result;
  GenericMethodCall call;
  call.scalarArg = static_cast<std::uint64_t>(n);
  call.hasScalarArg = true;
  it->second(call, nullptr, result);
  LargeAggregate agg = makeFfiBufFromBytes(result.bytes);
  FfiBufReturn out{};
  std::memcpy(&out, agg.bytes, sizeof(out));
  return out;
}

template <std::size_t SlotId>
void trampolineCompletionStatusBuf0(std::uint64_t handle, CompletionStatusBufFn complete, void* userdata) {
  auto obj = lookup(handle);
  auto onComplete = [complete, userdata](CallbackCompletion c) {
    LargeAggregate agg = makeFfiBufFromBytes(c.bytes);
    complete(userdata, c.statusCode, agg);
  };
  if (!obj) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  CallbackResult result;
  GenericMethodCall call;
  it->second(call, onComplete, result);
}

template <std::size_t SlotId>
void trampolineCompletionStatusBuf1(std::uint64_t handle, const std::uint8_t* ptr, std::uintptr_t len,
                                     CompletionStatusBufFn complete, void* userdata) {
  auto obj = lookup(handle);
  auto onComplete = [complete, userdata](CallbackCompletion c) {
    LargeAggregate agg = makeFfiBufFromBytes(c.bytes);
    complete(userdata, c.statusCode, agg);
  };
  if (!obj) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  auto it = obj->methods.find(slotMethodNames()[SlotId]);
  if (it == obj->methods.end()) {
    onComplete(CallbackCompletion{kFfiStatusCancelled, {}});
    return;
  }
  CallbackResult result;
  GenericMethodCall call;
  call.bufArgs.push_back(std::vector<std::uint8_t>(ptr, ptr + len));
  it->second(call, onComplete, result);
}

}  // namespace detail

/// Describes one vtable field this builder knows how to generate a generic trampoline for.
enum class CallbackShape {
  Free,
  Clone,
  ScalarReturn,
  VoidArgs0,
  VoidScalar1,
  VoidScalar2,
  VoidBuf1,
  VoidBuf2Scalar1Buf1,
  ScalarInBufOut,
  CompletionStatus0,
  CompletionStatus1,
  CompletionStatus2,
  CompletionStatusBuf0,
  CompletionStatusBuf1,
};

/// Builds one vtable slot's function pointer for `shape`, assigning it a fresh compile-time SlotId
/// bound (at this call) to `methodName`. Returns the raw function pointer to store in the vtable
/// struct's memory at the field's offset. `Free`/`Clone` need no `methodName` (they're universal).
void* buildCallbackTrampoline(CallbackShape shape, const std::string& methodName);

/// Classifies a parsed vtable field into one of the shapes this file knows how to generate a
/// trampoline for, purely from its param-kind sequence -- e.g. `(U64, PtrConst, U64, FnPtr, PtrMut)`
/// (an `int32_t`/`uintptr_t`-typed len always follows its `const uint8_t*`, matching every real
/// buffer-in param this header ever emits) classifies as `CompletionStatusBuf1`. Returns
/// `std::nullopt` for a shape this file doesn't cover yet (see the module doc's "NOT YET covered"
/// note) rather than guessing.
std::optional<CallbackShape> classifyVTableField(const VTableFieldAbi& field);

/// Builds a complete vtable's raw bytes (one pointer-sized slot per field, in declared order --
/// binary-identical to the real `typedef struct { ... }` layout since every field is a same-size,
/// same-alignment function pointer) for `vtable`, given each non-free/non-clone field's registered
/// JS-backed method name equals its own C field name (true of every real trait this session
/// modeled: `Multiplier::factor` -> vtable field `factor`; `AsyncKv::get`/`set` -> fields
/// `get`/`set`). Fields this file can't classify are left null (a call through them is a Rust-side
/// bug, never reached by a correctly-modeled trait) -- callers may assert `classifyVTableField`
/// succeeds for every field of a trait they intend to fully support.
std::vector<void*> buildVTableBytes(const VTableAbi& vtable);

/// Builds `vtable`'s trampoline slots and registers them for the REST OF THE PROCESS's lifetime,
/// returning the pointer to pass to `boltffi_register_callback_*`. This is the ONLY sanctioned way
/// to obtain that pointer for an actual registration call (as opposed to `buildVTableBytes`
/// directly, still useful for tests that never call `boltffi_register_callback_*` for real): the
/// real generated registration function stores this EXACT pointer in a process-wide
/// `static AtomicPtr` and dereferences it on EVERY SUBSEQUENT call into the trait for as long as
/// the process runs (verified against boltffi_macros' actual codegen,
/// `experimental/wrapper/callback.rs`'s `#register_ident`/`#foreign_vtable_static` -- there is no
/// copy on the Rust side, and no "unregister"/teardown call anywhere in the real ABI). A caller
/// that registers a plain, function-local `std::vector<void*>` and lets it go out of scope once
/// registration returns has handed Rust a pointer that is GUARANTEED dangling on the very next
/// call into that trait -- not a speculative race, a certainty, since Rust retains the pointer
/// forever. This intentionally leaks the slot storage (mirrors Rust's own choice of a `static`,
/// never freed) rather than returning ownership to a caller who has to remember an unusual
/// lifetime rule.
inline const void* registerVTableForProcessLifetime(const VTableAbi& vtable) {
  auto* slots = new std::vector<void*>(buildVTableBytes(vtable));
  return slots->data();
}

}  // namespace boltffi
