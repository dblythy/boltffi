# `runtime/cpp` — the react-native track's stage-3 JSI adapter core

See `docs/tracks/react-native.md` (parse-core-sdks repo) for the full design. This directory is
the fork-side deliverable: a reusable C++ core any boltffi-generated native artifact (Apple
xcframework, Android `.so`) can be driven through from a JSI `HostObject`, plus the first real
proof-of-mechanism slice against the `rn_poc` fixture
(`runtime/typescript/test/fixtures/rn_poc`).

## Layout

```
include/boltffi/native_trampoline.h   the async continuation trampoline (NO jsi:: dependency)
include/boltffi/ffi_buf.h             FfiBuf decode -> owned std::vector<uint8_t> (NO jsi:: dep)
src/native_trampoline.cpp
src/ffi_buf.cpp
jsi/boltffi_counter_host_object.h/.cpp   a real jsi::HostObject driving rn_poc's Counter/Multiplier
jsi/linkcheck_main.cpp                   compile+link+run proof against real jsi.h + rn_poc's dylib
tests/native_trampoline_test.cpp      pure C++ unit tests (registration races, off-thread
                                       delivery, teardown safety) -- no JSI, no Rust toolchain
tests/ffi_buf_test.cpp                pure C++ unit tests for the buffer decode
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

## Stage-4 remainder (honest, not deferred silently)

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
