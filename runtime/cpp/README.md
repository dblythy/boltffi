# `runtime/cpp` — the react-native track's JSI adapter core

See `docs/tracks/react-native.md` (parse-core-sdks repo) for the full design. This directory is
the fork-side deliverable: a reusable C++ core any boltffi-generated native artifact (Apple
xcframework, Android `.so`) can be driven through from a JSI `HostObject`. Stage 3 proved the
mechanism against one fixture class (`Counter`/`Multiplier`); the jsi-dispatch session generalized
it to a **name-driven dispatcher** (item A: resolve and call ARBITRARY `boltffi_*` symbols, no
per-symbol C++) and a **generic host-callback vtable builder** (item B: bridge
`SessionStorage`/`HttpTransport`/... style callbacks from Rust into JS), both proven against the
REAL `parse-core-rs` header/library in addition to the `rn_poc` fixture
(`runtime/typescript/test/fixtures/rn_poc`).

## Layout

```
include/boltffi/native_trampoline.h   the async continuation trampoline (NO jsi:: dependency)
include/boltffi/ffi_buf.h             FfiBuf decode -> owned std::vector<uint8_t> (NO jsi:: dep)
include/boltffi/abi_header.h          parses a boltffi-generated C header into a typed ABI table
                                       (item A's metadata source; NO jsi:: dep)
include/boltffi/generic_invoke.h      the no-libffi call mechanism over a small closed shape space
                                       (item A's call primitive; NO jsi:: dep)
include/boltffi/generic_callback.h    the generic callback-vtable trampoline library (item B; NO
                                       jsi:: dep)
src/native_trampoline.cpp
src/ffi_buf.cpp
src/abi_header.cpp
src/generic_callback.cpp
jsi/boltffi_counter_host_object.h/.cpp   the stage-3 fixture host object (rn_poc Counter/Multiplier)
jsi/boltffi_generic_host_object.h/.cpp   the item-A generic host object: get()/getPropertyNames()
                                          resolve any parsed-ABI function name generically, plus the
                                          `__boltffi_native_bind_arena` contract (see below)
jsi/linkcheck_main.cpp                   compile+link+run proof against real jsi.h + rn_poc's dylib
tests/native_trampoline_test.cpp      pure C++ unit tests (registration races, off-thread
                                       delivery, teardown safety) -- no JSI, no Rust toolchain
tests/ffi_buf_test.cpp                pure C++ unit tests for the buffer decode
tests/abi_header_test.cpp             parser unit tests, including a full parse of the REAL,
                                       unmodified 689-function parse-core-rs header (fixtures/)
tests/generic_invoke_test.cpp         the call mechanism's unit tests (arity/float-position/
                                       aggregate-return shapes)
tests/generic_callback_test.cpp       the callback-vtable builder's unit tests, including a sweep
                                       classifying every field of the real header's 13 vtables
tests/real_crate_integration_test.cpp end-to-end proof against a REAL compiled `rn_poc` dylib:
                                       generic sync/async dispatch + a generic host-callback vtable
                                       round trip (see "What's proven" below)
CMakeLists.txt
```

## Why two layers

`native_trampoline.h`/`ffi_buf.h` touch no `jsi::` type at all -- they are unit-testable with
nothing but a C++17 compiler (see "Building" below), and cover exactly the JSI-independent half
of the adapter's risk surface: the same-call synchronous "Waked" race, off-thread signal
delivery, and safe teardown with pending continuations. `jsi/boltffi_counter_host_object.*` is the
thin, mechanical `jsi::Value`<->native wrapping layer every method needs; it is compiled and
LINKED against the real `facebook::jsi` library (see below), and its JSI-independent core methods
(`addSync`/`delayedAddAsync`/`setMultiplier`/`scaledSync`) are exercised end-to-end against the
real compiled `rn_poc` dylib by `jsi/linkcheck_main.cpp` -- but `get()`/`getPropertyNames()`
themselves are never invoked without a live JS engine (see "Stage-4 remainder").

## Building

The JSI-independent core and its tests need nothing beyond CMake + a C++17 compiler:

```bash
mkdir build && cd build
cmake ..
cmake --build . -j4
ctest --output-on-failure   # boltffi_jsi_core_tests, boltffi_ffi_buf_tests
```

The JSI-coupled adapter additionally needs a real `facebook::jsi`/`CallInvoker` header (and
`jsi.cpp`/`jsilib-posix.cpp` source) checkout -- the cheapest source for these during development
is `node_modules/react-native/ReactCommon` in any RN project, or `npm pack react-native` and
extract `package/ReactCommon`:

```bash
cmake -DBOLTFFI_JSI_INCLUDE_DIR=/path/to/ReactCommon/jsi \
      -DBOLTFFI_CALLINVOKER_INCLUDE_DIR=/path/to/ReactCommon/callinvoker \
      ..
cmake --build . -j4
# build the rn_poc fixture once (see runtime/typescript/test/fixtures/build-rn-poc.sh), then:
./boltffi_jsi_adapter_linkcheck /path/to/librn_poc.dylib
```

This builds `facebook::jsi::Runtime`/`HostObject`/`Value`/`Function`/... from Meta's own
`jsi.cpp` + `jsilib-posix.cpp` (deliberately excluding `JSIDynamic.cpp`, the one file in that
directory that needs `folly::dynamic` -- this adapter never uses it), so the compile+link proof
is against the genuine JSI ABI, not a hand-rolled stub. `CallInvoker.h` is header-only (a pure
abstract interface) and needs no separate library.

Verified this session: `boltffi_jsi_adapter_linkcheck` compiles, links, and runs clean against a
real `rn_poc` dylib built through the real experimental `BindingExpansion` macro path -- all of
sync dispatch (`add`), the full native-async continuation protocol (`delayed_add`: register ->
off-thread wake -> repoll -> Ready -> real `CallInvoker::invokeAsync` hop -> complete -> free),
and the host-callback vtable (`set_multiplier`/`scaled`, a JS-implemented `Multiplier` Rust calls
back into) pass, repeatably.

## Class-method native-async codegen

The fork's stage-2 native-async dispatch mode (`async_function.txt`) only covered top-level
functions. Since the adapter's whole point is exposing `ParseClient` (a class) over this
protocol, class methods needed the same mode -- closed this session in
`boltffi_bindgen/src/render/typescript/{plan,lower,templates}.rs` and
`templates/render_typescript/{class,value_type_companion}.txt`: `TsClassAsyncMethod` and
`TsValueTypeAsyncMethod` gained the same `poll_ffi_name`/`native_async` pair `TsAsyncFunction`
already had, and both class-method and value-type-method async templates gained a
`native_async` branch mirroring `async_function.txt`'s (scalar/void supported, buffer-encoded
returns and wasm-alloc-needing params fail loudly, matching the free-function precedent).

Value-type (record) async methods are wired at the plan/template layer for parity, but cannot be
exercised end-to-end via `lower_contract` today: the ABI contract builder deliberately excludes
async record methods entirely (`crate::ir::lower::tests::to_abi_contract_excludes_async_record_methods`,
pre-existing, unrelated to this change) -- a separate, older gap, out of this session's scope.

## The jsi-dispatch session: item A (generic dispatcher) + item B (callback bridge)

**Design decision: no libffi.** Measured against the REAL header this fork ships for every native
target (`tests/fixtures/parse_core_real_abi.h`, captured from an actual `parse-core-rs` build: 689
functions, 13 callback vtables, 6 by-value records), the call-shape space is small and closed:
every argument is either register-class (bool/i32/u32/i64/u64/any pointer -- x86-64 SysV and
AArch64 AAPCS64 both pass these uniformly in a general-purpose register regardless of declared
width) or `double` (the only other scalar type present, always at most one per function). By-value
struct returns/params reduce to three known sizes (`FfiStatus` 4B, `BoltFFICallbackHandle` 16B,
`FfiBuf_u8`/`FfiString` >16B). `generic_invoke.h` and `generic_callback.h` cover this whole
measured space with compile-time-generated tables (`std::index_sequence`-driven, ~130 tiny
instantiations total) -- no new dependency, no per-symbol C++.

**The one real ABI bug found and fixed this session, empirically:** a first version emulated a
large (>16 byte) by-value struct return by casting the callee to `void(*)(OutPtr, args...)` and
passing the "hidden" sret pointer as a normal leading argument -- correct on x86-64 SysV (where
that IS the convention) but **crashes on AArch64**, which passes the indirect-result pointer in a
dedicated register (`x8`), never a normal argument register. Reproduced with both a synthetic local
function and a real `parse-core-rs` symbol (`boltffi_init_record_parse_core_ffi_acl_acl_new`)
before the fix; the fix declares the REAL aggregate C++ type as the return and lets the compiler
generate the platform-correct convention (`generic_invoke.h`'s `LargeAggregate`,
`generic_invoke_test.cpp`'s `large_aggregate_return_via_sret` pins the regression). The mirror-image
version of the same mistake (a completion callback's `FfiBuf_u8`-by-value PARAMETER) was caught the
same way before it ever reached a device.

**What's proven, against a REAL compiled `rn_poc` dylib** (built through the same experimental
`BindingExpansion` macro path `dist/apple`/`dist/android` use;
`tests/real_crate_integration_test.cpp`, 4/4 passing, stable across repeated runs):
- Item A: construct an object and call a sync method purely by name (no per-symbol C++); drive a
  real async method (`delayed_add`) through the full register/off-thread-wake/repoll/sret-complete
  protocol, generically.
- Item B: register a generic vtable for `Multiplier` (a synchronous `ScalarReturn`-shaped trait) and
  drive `set_multiplier`/`scaled` through it. Register a generic vtable for `AsyncKv` (added this
  session specifically to mirror `SessionStorage`'s real async-completion shape,
  `#[export] #[async_trait::async_trait] trait AsyncKv { async fn get(&self) -> Option<String>; async fn set(&self, value: String); }`)
  and drive `Counter::kv_get` — a Rust async method that itself `.await`s a host-implemented async
  trait method whose completion fires ~30ms later from a background thread — end to end, decoding
  the result through the REAL wire format (`Option<String>`: a tag byte, then a 4-byte
  little-endian length prefix and UTF-8 bytes, `boltffi_core::wire::encode`). This exercises BOTH
  async directions (Rust future continuation and host-callback completion) in one round trip.
- `generic_callback_test.cpp`'s classification sweep confirms `classifyVTableField` succeeds for
  every field of all 13 real vtables except two documented gaps (below).

**The generic host object** (`jsi/boltffi_generic_host_object.h/.cpp`) wraps both into a real
`facebook::jsi::HostObject`: `get()`/`getPropertyNames()` resolve any parsed function name and
marshal JS `number`/`bigint` arguments generically (compiled and linked against real `jsi.h`, same
"not exercised without a live JS engine" honesty as `boltffi_counter_host_object.h` -- see "Stage-4
remainder" below, which still applies here unchanged). It also implements the arena-binding
contract this session's coordinator update added on the TypeScript side
(`@boltffi/runtime`'s `native.ts`/`native_arena.ts`): the generated TS treats every pointer-shaped
argument as an OFFSET into a pure-JS-simulated arena, calling a reserved
`exports.__boltffi_native_bind_arena(buffer)` once at construction and again on every arena growth
(which replaces the backing `ArrayBuffer`). `bindArena()` stores the real base address; every call
RE-DERIVES it (never caches across calls) and adds it to any `PtrConst`/`PtrMut`-classified
argument before dispatching -- caching would read freed memory the instant JS grows the arena, the
exact HIGH-severity bug the coordinator's own fix closed on the TypeScript side.

### A genuinely new finding: the generated TS and the real native artifacts use TWO DIFFERENT ABIs

Found while tracing exactly which symbol names the generic dispatcher should resolve. `boltffi
generate typescript` (what `scripts/build-react-native.sh` runs, with or without the
`native_async` experimental flag) emits calls against SHORT symbol names
(`boltffi_parse_client_new`) -- the STABLE macro path's naming, the same one a plain `cargo build`
produces (confirmed: `boltffi_bindgen`'s `entry_ffi_name = abi_call.symbol.as_str()` for the
`typescript` render surface resolves to this scheme). But `dist/apple`/`dist/android` (what a real
RN app actually links) are built via the EXPERIMENTAL `BindingExpansion` macro path
(`docs/tracks/boltffi-fork.md`'s own "Integration — landed" note), which uses LONG,
module-qualified names (`boltffi_init_class_parse_core_ffi_client_parse_client_new`) and real typed
signatures (verified directly against the real header this session:
`tests/fixtures/parse_core_real_abi.h`) -- **no symbol exported by the real artifacts matches what
the generated TypeScript calls today.** This is upstream of anything in `runtime/cpp`: either a
future session teaches `typescript`+`native_async` codegen to target the `BindingExpansion` naming
(the artifacts a real device build actually links), or RN gets its own build path compiling
`parse-core-rs` through the STABLE macro path for a real (non-wasm) target triple, matching what
the generated TS already expects. This adapter's dispatcher itself is naming-scheme-agnostic (it
resolves whatever string name it's asked for via `dlsym`) — this finding is about which artifact a
production consumer must link, not a limitation of the mechanism proven here.

### Two documented, small callback shapes not yet covered

`generic_callback.h`'s trampoline library covers 12 of the real header's 13 vtables IN FULL. Two
real shapes are NOT yet generated (neither is exercised by any trait this session's fixture proof
needs, and both are single, mechanical additions following the exact pattern already established):
- `EventuallyQueueListener::on_dropped`'s `(ptr,len,ptr,len,i32,ptr,len)` shape (3 buffers + 1
  scalar, sync void) — a straightforward `VoidBuf`-family extension.
- `RandomSource::fill`'s `FfiBuf_u8(uint64_t, uint32_t)` shape (scalar-in, large-aggregate-out) —
  needs its own exact-32-byte C++ struct as the trampoline's return type (a callback trampoline is
  the ABI's CALLEE, so — unlike `generic_invoke.h`'s caller-side `LargeAggregate<64>` — it must
  write EXACTLY `sizeof(FfiBuf_u8)` bytes, not an over-sized cap type, or it overflows the real
  caller's (Rust's) allocation).

## Stage-4 remainder (honest, not deferred silently)

- **No live JS engine here, still.** `get()`/`getPropertyNames()` on BOTH host objects are
  compiled and linked against real `jsi.h`/`jsi.cpp`, but never CALLED through a live
  `jsi::Runtime` (Hermes/JSC) in this session either — the same constraint the stage-3 note
  described, unchanged by this session's generic dispatcher. The test plan is the same: a real
  Hermes checkout (or Nitro's own harness), install the host object as a real global property,
  drive it from real JS source strings.
- **Packaging is still a skeleton, not a built artifact** — unchanged from stage 3.
- **The generic dispatcher does not yet expose async methods or callback registration through
  `get()`'s live `jsi::Value` surface** — both are fully designed and proven at the JSI-independent
  layer (`native_trampoline.h` + `generic_callback.h`, exercised end-to-end in
  `real_crate_integration_test.cpp` against the real `rn_poc` crate), but `boltffi_generic_
  host_object.cpp`'s `get()` only wires the plain-sync-call path (plus `__boltffi_native_bind_
  arena`) into live `jsi::Function`s today. Wiring a `Promise`-returning async path and a callback-
  registration entry point through `get()` is the next incremental step, not a new mechanism.
- **No-copy `jsi::ArrayBuffer` for buffer-encoded returns** is still intentionally deferred (eager
  copy-then-free, unchanged from stage 3's decision) — the generic host object's large-aggregate
  return path copies into an owned `std::vector` and wraps THAT in a `jsi::ArrayBuffer`.
- **The TS/native-ABI naming mismatch** (see above) needs a decision and a fix in a future session
  before ANY real `ParseClient` call — generated by today's `native_async` TypeScript — can
  actually reach a real device artifact through this (or any) JSI adapter.

## Stage-4 remainder, original (stage 3, still true for what it covers)

- **No live JS engine here.** `get()`/`getPropertyNames()` -- the actual JSI property-access
  entry points a real `HostObject` is invoked through -- are compiled and linked against real
  `jsi.h`/`jsi.cpp`, but never CALLED in this stage: doing so needs a concrete `jsi::Runtime`
  (Hermes or JSC), which this stage deliberately does not vendor (matches the mandate's
  "compile-and-link proof... is this stage's bar; a full RN app build is stage 4"). The stage-4
  test plan: build against a real Hermes checkout (or Nitro's own harness, which already wires
  Hermes for host-side testing), instantiate `BoltFFICounterHostObject` as a real global property,
  and drive it from actual JS source strings (`rt.evaluateJavaScript(...)`) -- exercising
  `get()`/the `Promise` executor/the `jsi::Function` callback path for real.
- **Packaging is a skeleton, not a built artifact.** `boltffi-jsi-adapter.podspec` and the
  Android CMake/prefab notes (see `packaging/`) describe how a consuming RN library (e.g.
  parse-core-sdks's future `sdks/react-native/`) would bundle this adapter alongside its own
  compiled xcframework/`.so` -- neither was built against a real RN app target this session (no
  Xcode RN project, no Android NDK toolchain confirmed in this environment); that build-and-link
  proof against a real RN host app is stage 4's packaging half.
- **The generic call-by-name dispatch primitive** (`BoltFFIExports`'s shape, covering all ~80
  `ParseClient` methods generically) is NOT here -- this stage proves the mechanism against one
  fixture class (`Counter`/`Multiplier`, mirroring the design doc's own "first slice" scope:
  construction, one sync call, one async call, one host-callback round trip). Generalizing to a
  name-driven dispatcher over `dlsym`-resolved symbols, for every method shape the codegen emits,
  is stage 4's "full parity" item.
- **No-copy `jsi::ArrayBuffer` for buffer-encoded returns** is intentionally deferred (eager
  copy-then-free is this stage's default, per the design doc's own risk-table entry on
  `cap`/`align` retention hazards) -- `ffi_buf.h`'s `decodeAndFreeFfiBuf` is the only buffer path
  implemented; wiring its output into a `jsi::ArrayBuffer`/`MutableBuffer` is one more file,
  deferred as a specifically-reviewed follow-up per the design doc.
