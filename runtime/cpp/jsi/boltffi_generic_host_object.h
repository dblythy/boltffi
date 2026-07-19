// The react-native track's item A deliverable: a name-driven `jsi::HostObject` whose `get()`
// resolves ARBITRARY `boltffi_*` symbols from a loaded native artifact and calls them generically
// (`boltffi/generic_invoke.h`) -- no per-symbol C++, extending the mechanism
// `real_crate_integration_test.cpp` proves against the real `rn_poc` crate up into the actual JSI
// boundary. Mirrors `boltffi_counter_host_object.h`'s split: this file's `get()`/`getPropertyNames()`
// are real, compiled and linked against genuine `facebook::jsi` types, but -- like that file --
// never exercised through a live `jsi::Runtime` in this stage (no Hermes/JSC vendored here; see
// runtime/cpp/README.md's Stage-4-remainder note, which applies identically here).
//
// The arena-binding contract (`__boltffi_native_bind_arena`, `@boltffi/runtime`'s
// `native.ts`/`native_arena.ts`, this session's coordinator update): the generated TS's
// `NativeBoltFFIModule` treats every "pointer" it hands across the boundary as an OFFSET into a
// pure-JS-simulated arena (`NativeMemoryArena`), not a real address -- it calls
// `exports.__boltffi_native_bind_arena(buffer)` once at construction and again on every arena
// GROWTH (which replaces the backing `ArrayBuffer`). This HostObject exposes that reserved
// property: `bindArena` stores the `jsi::ArrayBuffer` handle (a cheap, ref-counted wrapper -- the
// real bytes live in the JS engine's heap) and RE-DERIVES its `data(rt)` pointer on every single
// generic call, never caching it across calls -- caching would read freed memory the instant JS
// grows the arena and gets a new backing buffer (the exact HIGH bug the coordinator's update
// names). `translateArenaOffset` is the one place a `PtrConst`/`PtrMut`-classified argument's JS
// `number` (an arena offset) becomes a real address for the call.
#pragma once

#include <jsi/jsi.h>

#include <atomic>
#include <cstdint>
#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>

#include "boltffi/abi_header.h"
#include "boltffi/generic_invoke.h"

namespace boltffi {

class BoltFFIGenericHostObject : public facebook::jsi::HostObject {
 public:
  /// `dylibHandle` is already dlopen'd/linked by the caller (this class never dlopens anything
  /// itself, matching the packaging doc's "link at build time, never dlopen at app runtime"
  /// decision -- `dylibHandle` may just as well be `RTLD_DEFAULT` for a statically-linked artifact).
  /// `abi` is the result of `parseAbiHeader` against the SAME artifact's shipped header.
  BoltFFIGenericHostObject(void* dylibHandle, ParsedAbi abi);

  facebook::jsi::Value get(facebook::jsi::Runtime& rt, const facebook::jsi::PropNameID& name) override;
  std::vector<facebook::jsi::PropNameID> getPropertyNames(facebook::jsi::Runtime& rt) override;

  /// The JSI-INDEPENDENT core of a plain (non-async, non-callback-registration) function call:
  /// resolves `name` in the ABI table, translates each `CValue`'s `Ptr`-kind slots through the
  /// currently-bound arena (if any), and dispatches via `generic_invoke.h`. Returns the raw 64-bit
  /// result (or writes an aggregate's bytes into `aggregateOut` when the return is `>16` bytes) --
  /// the JSI-coupled `get()` wraps this into a `jsi::Value`/`jsi::Function`; exposed directly here
  /// so it's testable without a live `jsi::Runtime` (mirrors `BoltFFICounterHostObject`'s own
  /// core/wrapper split).
  std::uint64_t callScalar(const std::string& name, const std::vector<CValue>& args);
  void callSretIntoBuffer(const std::string& name, const std::vector<CValue>& args, void* outBuffer,
                           std::size_t outSize);

  /// Stores `buffer`'s real backing address as the arena's current base -- called once at JS-side
  /// construction and again on every arena grow (see the module doc). `baseAddress` is a real
  /// process address for the duration this HostObject is alive to translate against it; a real
  /// adapter re-derives it via `jsi::ArrayBuffer::data(rt)` (this pure-C++-testable overload takes
  /// the address directly so it's exercisable without a live `jsi::Runtime`).
  void bindArena(std::uint8_t* baseAddress);

  const ParsedAbi& abi() const { return abi_; }

 private:
  void* resolveSymbol(const std::string& name);
  CValue translateIfPointer(const TypeRef& paramType, CValue value) const;

  void* dylib_;
  ParsedAbi abi_;
  std::mutex symbolsMutex_;
  std::unordered_map<std::string, void*> resolvedSymbols_;
  std::atomic<std::uint8_t*> arenaBase_{nullptr};
};

}  // namespace boltffi
