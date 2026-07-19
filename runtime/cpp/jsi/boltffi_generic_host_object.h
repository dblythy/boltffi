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
// names). `translateIfPointer` (via `abi_header.h`'s `isArenaPointerKind`) is the one place a
// REAL inline-pointer-classified argument's JS `number` (an arena offset) becomes a real address
// for the call -- an `OpaqueHandle`-classified argument (e.g. `RustFutureHandle`) is explicitly
// NEVER rebased (finding 3, this session): it crosses verbatim, since it is a real Rust-owned
// pointer already, not an offset into anything this side owns.
//
// Argument/return shapes wider than one register (findings 1+2, this session): a logical
// parameter/return classified `Aggregate` by the ABI parser is either the one 16-byte
// `BoltFFICallbackHandle` shape (`planFunctionCall`/`planFunctionReturn` in `abi_header.h`) or
// outside this dispatcher's closed shape space entirely (`boltffi_free_string`/`boltffi_free_buf`'s
// own >16-byte by-value parameters) -- the latter is REJECTED at `get()` time (the property simply
// doesn't resolve, exactly like an unknown function name) rather than silently mis-called. Both a
// 16-byte return (`BoltFFICallbackHandle`, e.g. every `boltffi_create_callback_*`) and a >16-byte
// return (`FfiBuf_u8`/`FfiString`) cross into JS as a plain `ArrayBuffer` of their exact byte size
// (the SAME convention, just sized differently) -- a 16-byte `BoltFFICallbackHandle` ARGUMENT is
// symmetrically expected as that exact ArrayBuffer shape, so a value `create_callback_*` just
// returned can be handed straight back into a setter like `set_http_transport` unmodified.
//
// Reentrant arena growth (finding 8, this session): a SYNCHRONOUS host-callback shape (e.g.
// `Multiplier::factor`) can, in principle, call back into JS before the outer native call this
// HostObject dispatched returns -- and that reentrant JS code could grow the arena (replacing its
// backing `ArrayBuffer`) while the OUTER call's already-translated pointer arguments still point
// into the OLD one. `currentArenaBuffer_` holds the actual `jsi::ArrayBuffer` object (not just its
// raw address, which is all `arenaBase_`/`bindArena` track for JSI-INDEPENDENT testability); each
// generic call PINS a local copy of it for the call's entire duration before doing any pointer
// translation, keeping the backing store alive (via JSI's own refcounting) even if a reentrant
// `__boltffi_native_bind_arena` call replaces the MEMBER copy mid-call.
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
  /// resolves `name` in the ABI table and dispatches `registerArgs` (already expanded to
  /// register-level slots and pointer-translated by the caller, see `buildRegisterArgs`) via
  /// `generic_invoke.h`. Returns the raw 64-bit result -- the JSI-coupled `get()` wraps this into a
  /// `jsi::Value`/`jsi::Function`; exposed directly here so it's testable without a live
  /// `jsi::Runtime` (mirrors `BoltFFICounterHostObject`'s own core/wrapper split).
  std::uint64_t callScalar(const std::string& name, const std::vector<CValue>& registerArgs);
  /// The `ReturnPlan::Sret` counterpart (return wider than 16 bytes, e.g. `FfiBuf_u8`/`FfiString`).
  void callSretIntoBuffer(const std::string& name, const std::vector<CValue>& registerArgs, void* outBuffer,
                           std::size_t outSize);
  /// The `ReturnPlan::TwoWord` counterpart (exactly the 16-byte `BoltFFICallbackHandle` return
  /// shape, e.g. every `boltffi_create_callback_*`) -- finding 2's fix: before this, EVERY
  /// aggregate return other than `> 16` bytes fell through to `callScalar`, silently losing the
  /// `vtable` word for this exact shape.
  TwoWord callTwoWord(const std::string& name, const std::vector<CValue>& registerArgs);

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

  /// The JSI-COUPLED counterpart of `arenaBase_`: a heap-held handle to the actual
  /// `jsi::ArrayBuffer` object currently bound, so a call can PIN a local copy of the
  /// `shared_ptr` itself (keeping the underlying buffer alive via JSI's own refcounting for as
  /// long as ANY `shared_ptr` to it survives) for its entire duration -- see the module doc's
  /// finding-8 note. `shared_ptr` copy is used deliberately instead of copying the
  /// `jsi::ArrayBuffer` value directly: `jsi::Pointer` (its base class) is move-only, so a
  /// `shared_ptr<ArrayBuffer>` is the straightforward way to hold one MORE-THAN-ONE-owner
  /// reference to it. Only ever touched from `get()`'s real (jsi::Runtime&-bearing) code path;
  /// `bindArena`'s pure-C++-testable overload never sees it, matching that overload's own
  /// "exercisable without a live Runtime" contract.
  std::mutex arenaObjectMutex_;
  std::shared_ptr<facebook::jsi::ArrayBuffer> currentArenaBuffer_;
};

}  // namespace boltffi
