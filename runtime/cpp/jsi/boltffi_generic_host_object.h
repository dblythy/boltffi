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
// property: it stores the CURRENT `jsi::ArrayBuffer` handle (`currentArenaBuffer_`, a cheap,
// ref-counted wrapper -- the real bytes live in the JS engine's heap) and RE-DERIVES its `data(rt)`
// pointer on every single generic call from a LOCAL PIN of that same handle, never caching a raw
// address across calls -- caching would read freed memory the instant JS grows the arena and gets
// a new backing buffer (the exact HIGH bug the coordinator's update names). `translateIfPointer`
// (via `abi_header.h`'s `isArenaPointerKind`) is the one place a REAL inline-pointer-classified
// argument's JS `number` (an arena offset) becomes a real address for the call, given that call's
// own pinned base -- an `OpaqueHandle`-classified argument (e.g. `RustFutureHandle`, or a
// `boltffi_register_callback_*` vtable pointer, finding 1 of the second adversarial round) is
// explicitly NEVER rebased (finding 3, first session): it crosses verbatim, since it is a real
// process-owned pointer already, not an offset into anything this side owns.
//
// One base, derived once, from the SAME pin (finding 1 of the second adversarial round's OTHER
// residual: a torn snapshot): an earlier revision tracked the base address in a SEPARATE
// `std::atomic<uint8_t*>` (`arenaBase_`, updated via a `bindArena` setter) alongside
// `currentArenaBuffer_` (guarded by `arenaObjectMutex_`) -- two independently-updated pieces of
// state a concurrent `__boltffi_native_bind_arena` rebind could tear apart (observing the NEW
// buffer object paired with the OLD base address, or vice versa). There is no such thing to tear
// anymore: a call pins `currentArenaBuffer_` ONCE under `arenaObjectMutex_` and derives its base
// address from that SAME pinned object (`pinnedArena->data(rt)`) -- the two facts can never
// disagree because they are now one fact.
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
// Reentrant arena growth (finding 8, first session): a SYNCHRONOUS host-callback shape (e.g.
// `Multiplier::factor`) can, in principle, call back into JS before the outer native call this
// HostObject dispatched returns -- and that reentrant JS code could grow the arena (replacing its
// backing `ArrayBuffer`) while the OUTER call's already-translated pointer arguments still point
// into the OLD one. `currentArenaBuffer_` holds the actual `jsi::ArrayBuffer` object; each generic
// call PINS a local copy of it for the call's entire duration before doing any pointer
// translation, keeping the backing store alive (via JSI's own refcounting) even if a reentrant
// `__boltffi_native_bind_arena` call replaces the MEMBER copy mid-call. Pinning fixes the
// object's LIFETIME, not the WRITE's coherence -- an out-param pointer computed against the
// pinned (possibly now-stale) buffer, written to by Rust after a reentrant grow, lands in memory
// JS is no longer looking at once it reads `arena.buffer` post-call (the second adversarial
// round's finding 2). That coherence half of the fix lives on the JS side
// (`native_arena.ts`'s call-in-flight growth guard -- see its module doc); this file only owns
// eliminating the (separate) torn-snapshot hazard described above.
//
// Why this file has no C++-side mirror of `beginCall()`/`endCall()`: this class cannot enforce
// the growth-coherence invariant itself, no matter what counter it kept, because it never causes
// growth and only ever LEARNS about one after the fact. Growth is 100% a JS-side event --
// `NativeMemoryArena.grow()` reallocates `this.buf`/`this.view`/`this.bytes` FIRST and only THEN
// invokes `onGrow`, which is what eventually calls back into THIS file's own
// `__boltffi_native_bind_arena` handler above. By the time that handler runs, the old buffer is
// already orphaned; a C++ counter refusing the rebind at that point could not undo the
// reallocation JS already performed, so it would neither prevent the coherence break nor even
// reliably detect it before JS itself does (`NativeMemoryArena.grow()`'s own `callDepth` check
// throws first, synchronously, from the same JS call stack that would otherwise have triggered
// this rebind). Adding a parallel depth counter here would be pure dead weight -- state that can
// never be the one to catch the bug, tracking something the JS side already tracks and already
// acts on. The invariant instead depends entirely on JS marking the call BEFORE it ever reaches
// this HostObject: `@boltffi/runtime`'s `native.ts` wires `beginCall()`/`endCall()` around its own
// three native-dispatch funnels (`NativeAsyncFutureManager.dispatchPoll`,
// `NativeBoltFFIModule.completeAsync`, `takeLastErrorMessage`) -- every generated async
// completion/poll/last-error call site routes through one of those three. What remains OPEN (a
// known gap, not silently accepted): a plain non-async call a generated `function.txt` function
// makes directly against `_exports.ffiName(...)` -- which, for a real JSI adapter, dispatches
// through THIS class's `get()` lambda exactly like any other call -- has no JS-side wrapper
// bracketing it at all today, so a synchronous host-callback such a call triggers could still grow
// the arena reentrantly underneath it. Closing that needs a codegen change (routing those calls
// through a marked `_module` wrapper too), not anything this file could add on its own.
#pragma once

#include <jsi/jsi.h>

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

  const ParsedAbi& abi() const { return abi_; }

 private:
  void* resolveSymbol(const std::string& name);
  /// `base` is the CURRENT call's pinned arena base (`pinnedArena->data(rt)`, computed once in
  /// `get()`'s lambda from the SAME buffer object the call pinned) -- passed explicitly rather
  /// than read from shared member state so there is exactly one source of truth per call and no
  /// separate atomic to tear against a concurrent rebind (the second adversarial round's finding 1
  /// residual; see the module doc). `base == nullptr` means no arena has ever been bound.
  CValue translateIfPointer(const TypeRef& paramType, CValue value, std::uint8_t* base) const;

  void* dylib_;
  ParsedAbi abi_;
  std::mutex symbolsMutex_;
  std::unordered_map<std::string, void*> resolvedSymbols_;

  /// The arena's current backing buffer, as a heap-held handle to the actual `jsi::ArrayBuffer`
  /// object (not a raw address -- a call derives its own address from a PIN of this same object,
  /// see `translateIfPointer`'s doc) so a call can PIN a local copy of the `shared_ptr` itself
  /// (keeping the underlying buffer alive via JSI's own refcounting for as long as ANY
  /// `shared_ptr` to it survives) for its entire duration -- see the module doc's finding-8 note.
  /// `shared_ptr` copy is used deliberately instead of copying the `jsi::ArrayBuffer` value
  /// directly: `jsi::Pointer` (its base class) is move-only, so a `shared_ptr<ArrayBuffer>` is the
  /// straightforward way to hold one MORE-THAN-ONE-owner reference to it.
  std::mutex arenaObjectMutex_;
  std::shared_ptr<facebook::jsi::ArrayBuffer> currentArenaBuffer_;
};

}  // namespace boltffi
