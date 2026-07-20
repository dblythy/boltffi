// Stage 1 (docs/tracks/react-native.md, parse-core-sdks repo) acceptance evidence: drives the
// REAL @boltffi/runtime native backend (src/native.ts) against a real compiled dylib (test
// fixtures/rn_poc, built through the actual experimental BindingExpansion macro path -- the same
// path apple/android/kmp/csharp build their shipped artifacts through, per
// docs/tracks/boltffi-fork.md's naming/calling-convention findings -- never a plain `cargo
// build`), through Bun's real FFI (`bun:ffi`'s `dlopen`/`JSCallback`), standing in for the
// eventual JSI C++ adapter exactly as the design doc's own PoC did.
//
// Run with `bun test test/native.bun.test.ts` (NOT part of `npm test`/vitest -- Bun's FFI has no
// vitest/Node equivalent, and this suite requires a real `cargo`/rustc toolchain plus a
// Rust-compiled dylib, unlike the wasm-backed vitest suite's hand-assembled byte fixtures. Kept as
// a separate, explicitly-invoked suite so `npm test`'s 22 existing tests stay exactly as they
// were -- this file adds new coverage, it does not touch or gate the old suite).
import { describe, test, expect, beforeAll, afterAll } from "bun:test";
import { dlopen, FFIType, JSCallback, ptr, type Pointer } from "bun:ffi";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";

import {
  instantiateBoltFFINative,
  NativeContinuationSignal,
  NativeFfiStatus,
  allocNativeStatusBuffer,
  readNativeStatusCode,
  type NativeHandle,
} from "../src/native.js";
import {
  bootstrapCallbackVTable,
  NativeCallbackTraitRegistry,
  _resetCallbackVTableRegistrationsForTests,
} from "../src/native_callback.js";

const FIXTURE_DIR = new URL("./fixtures", import.meta.url).pathname;
const DYLIB_PATH = `${FIXTURE_DIR}/rn_poc/target/release/librn_poc.dylib`;

function buildFixtureIfNeeded(): string {
  if (!existsSync(DYLIB_PATH)) {
    execFileSync(`${FIXTURE_DIR}/build-rn-poc.sh`, { stdio: "inherit" });
  }
  return DYLIB_PATH;
}

type NativeExports = ReturnType<typeof bindExports>;

function bindExports(path: string) {
  return dlopen(path, {
    boltffi_init_class_rn_poc_counter_new: { args: [FFIType.i32], returns: FFIType.u64 },
    boltffi_release_class_rn_poc_counter: { args: [FFIType.u64], returns: FFIType.void },
    boltffi_method_class_rn_poc_counter_add: {
      args: [FFIType.u64, FFIType.i32],
      returns: FFIType.i32,
    },
    boltffi_method_class_rn_poc_counter_delayed_add: {
      args: [FFIType.u64, FFIType.i32],
      returns: FFIType.ptr,
    },
    boltffi_async_method_class_rn_poc_counter_delayed_add_poll: {
      args: [FFIType.ptr, FFIType.u64, FFIType.function],
      returns: FFIType.void,
      threadsafe: true,
    },
    boltffi_async_method_class_rn_poc_counter_delayed_add_complete: {
      args: [FFIType.ptr, FFIType.ptr],
      returns: FFIType.i32,
    },
    boltffi_async_method_class_rn_poc_counter_delayed_add_cancel: {
      args: [FFIType.ptr],
      returns: FFIType.void,
    },
    boltffi_async_method_class_rn_poc_counter_delayed_add_free: {
      args: [FFIType.ptr],
      returns: FFIType.void,
    },
    boltffi_method_class_rn_poc_counter_scaled: { args: [FFIType.u64], returns: FFIType.i32 },
    boltffi_method_class_rn_poc_counter_set_multiplier: {
      // BoltFFICallbackHandle{ handle: u64, vtable: *const void } decomposed into two scalar
      // args -- see native.ts's module header for why this is ABI-safe for a <=16-byte,
      // all-integer-class struct rather than a hack.
      args: [FFIType.u64, FFIType.u64, FFIType.ptr],
      returns: FFIType.i32,
    },
    boltffi_register_callback_rn_poc_multiplier: { args: [FFIType.ptr], returns: FFIType.void },
  }).symbols;
}

describe("native backend PoC (rn_poc fixture, real dylib via Bun FFI)", () => {
  let exports: NativeExports;
  let trampoline: JSCallback;
  let module_: ReturnType<typeof instantiateBoltFFINative<NativeExports, Pointer>>;

  beforeAll(() => {
    const path = buildFixtureIfNeeded();
    exports = bindExports(path);
    module_ = instantiateBoltFFINative(exports, (onSignal) => {
      // The ONE generic continuation trampoline every async call in this test suite reuses --
      // `threadsafe: true` is load-bearing, not defensive boilerplate: the real Rust waker fires
      // this from a background OS thread (see the `delayed_add` test below), and Bun's own docs
      // state a non-threadsafe JSCallback invoked off Bun's own thread corrupts its closure. This
      // mirrors the design doc's "always hop through the engine's sanctioned thread-marshaling
      // primitive first" rule -- `CallInvoker::invokeAsync` for the eventual JSI adapter, this
      // flag for Bun.
      const cb = new JSCallback(
        (callbackData: bigint, signal: number) => onSignal(callbackData, signal as NativeContinuationSignal),
        { args: [FFIType.u64, FFIType.i8], returns: FFIType.void, threadsafe: true }
      );
      trampoline = cb;
      return cb.ptr as Pointer;
    });
  });

  afterAll(() => {
    trampoline?.close();
  });

  test("sync method call: Counter::add", () => {
    const handle = exports.boltffi_init_class_rn_poc_counter_new(10);
    try {
      expect(exports.boltffi_method_class_rn_poc_counter_add(handle, 5)).toBe(15);
      expect(exports.boltffi_method_class_rn_poc_counter_add(handle, 2)).toBe(17);
    } finally {
      exports.boltffi_release_class_rn_poc_counter(handle);
    }
  });

  test("async method call: Counter::delayed_add wakes off-thread and completes", async () => {
    const handle = exports.boltffi_init_class_rn_poc_counter_new(100);
    try {
      const future = exports.boltffi_method_class_rn_poc_counter_delayed_add(handle, 23) as unknown as NativeHandle;

      const awaitedFuture = await module_.asyncManager.pollAsyncNative(future, (h, callbackData, callback) => {
        exports.boltffi_async_method_class_rn_poc_counter_delayed_add_poll(
          h as unknown as Pointer,
          callbackData,
          callback
        );
      });
      expect(awaitedFuture).toBe(future);

      const statusBuf = allocNativeStatusBuffer();
      const result = exports.boltffi_async_method_class_rn_poc_counter_delayed_add_complete(
        awaitedFuture as unknown as Pointer,
        statusBuf
      );
      exports.boltffi_async_method_class_rn_poc_counter_delayed_add_free(awaitedFuture as unknown as Pointer);

      expect(readNativeStatusCode(statusBuf)).toBe(NativeFfiStatus.Ok);
      expect(result).toBe(123);
    } finally {
      exports.boltffi_release_class_rn_poc_counter(handle);
    }
  });

  test("host callback round trip: Rust calls a JS-implemented Multiplier", () => {
    const multipliers = new Map<bigint, () => number>();

    const freeCb = new JSCallback((handle: bigint) => void multipliers.delete(handle), {
      args: [FFIType.u64],
      returns: FFIType.void,
    });
    const cloneCb = new JSCallback(() => 0n, { args: [FFIType.u64], returns: FFIType.u64 });
    const factorCb = new JSCallback(
      (handle: bigint) => {
        const impl = multipliers.get(handle);
        return impl ? impl() : 1;
      },
      { args: [FFIType.u64], returns: FFIType.i32 }
    );

    // VTable field order (free, clone, factor) matches the generated struct layout
    // (Multiplier.cs's `VTable { free; clone; factor; }`) -- BoltFFI attaches the standard
    // free/clone lifecycle pair before a callback trait's own declared methods.
    const vtableBuf = new Uint8Array(24);
    const vtableView = new DataView(vtableBuf.buffer);
    vtableView.setBigUint64(0, BigInt(freeCb.ptr as unknown as number), true);
    vtableView.setBigUint64(8, BigInt(cloneCb.ptr as unknown as number), true);
    vtableView.setBigUint64(16, BigInt(factorCb.ptr as unknown as number), true);

    try {
      exports.boltffi_register_callback_rn_poc_multiplier(vtableBuf);

      const handle = exports.boltffi_init_class_rn_poc_counter_new(10);
      try {
        const callbackHandleId = 7n;
        multipliers.set(callbackHandleId, () => 4);

        const status = exports.boltffi_method_class_rn_poc_counter_set_multiplier(
          handle,
          callbackHandleId,
          ptr(vtableBuf)
        );
        expect(status).toBe(NativeFfiStatus.Ok);
        expect(exports.boltffi_method_class_rn_poc_counter_scaled(handle)).toBe(40);
      } finally {
        exports.boltffi_release_class_rn_poc_counter(handle);
      }

      // Rust's Box<dyn Multiplier> drop calls the vtable's `free` during release above.
      expect(multipliers.has(7n)).toBe(false);
    } finally {
      freeCb.close();
      cloneCb.close();
      factorCb.close();
    }
  });

  // The SAME round trip as the test above, but driven through `native_callback.ts`'s generic
  // `bootstrapCallbackVTable`/`NativeCallbackTraitRegistry` instead of the hand-rolled vtable
  // bytes/registry the previous test builds inline -- proves those primitives are genuinely
  // sufficient to drive a REAL callback trait against the real compiled dylib, end to end,
  // through Bun's own native-callback primitive (`JSCallback`) standing in for whatever a real
  // JSI adapter's `CallInvoker`-backed trampoline factory would supply. This is the design this
  // session's react-native native-mode host-callback-registration work generalizes for codegen to
  // emit automatically per callback trait (see native_callback.ts's own module doc).
  test("host callback round trip via native_callback.ts's generic registration primitives", () => {
    _resetCallbackVTableRegistrationsForTests();
    const registry = new NativeCallbackTraitRegistry<{ factor(): number }>();
    const tokens: JSCallback[] = [];

    // The host's own "make a real native function pointer from a JS closure" primitive --
    // Bun's `JSCallback` here, a JSI `CallInvoker`-backed C++ trampoline in production. Free/clone
    // are the two slots EVERY trait shares (mirrors `callback.txt`'s wasm-mode `_release`/
    // `_retain`, and `generic_callback.h`'s universal `genericFree`/`genericClone` on the C++
    // side); `factor` is `Multiplier`'s own one declared method.
    const freeToken = new JSCallback((handle: bigint) => void registry.release(handle), {
      args: [FFIType.u64],
      returns: FFIType.void,
    });
    const cloneToken = new JSCallback((handle: bigint) => registry.retain(handle), {
      args: [FFIType.u64],
      returns: FFIType.u64,
    });
    const factorToken = new JSCallback(
      (handle: bigint) => {
        const impl = registry.lookup(handle);
        return impl.factor();
      },
      { args: [FFIType.u64], returns: FFIType.i32 }
    );
    tokens.push(freeToken, cloneToken, factorToken);

    // Kept alive for the vtable's whole process lifetime (matching the real thing:
    // `registerVTableForProcessLifetime`'s own C++ discipline never frees its slot storage either)
    // -- a Bun `Pointer` handed to Rust does not itself keep the backing `Uint8Array` reachable to
    // Bun's GC, so a buffer built only inside `writeVTableBytes`'s closure and never referenced
    // again would risk collection while Rust still holds the raw address.
    let vtableBuf: Uint8Array;
    const writeVTableBytes = (fieldPtrs: readonly bigint[]): Pointer => {
      vtableBuf = new Uint8Array(fieldPtrs.length * 8);
      const view = new DataView(vtableBuf.buffer);
      fieldPtrs.forEach((fieldPtr, i) => view.setBigUint64(i * 8, fieldPtr, true));
      return ptr(vtableBuf);
    };

    try {
      bootstrapCallbackVTable(
        exports as unknown as Record<string, unknown>,
        "boltffi_register_callback_rn_poc_multiplier",
        [
          BigInt(freeToken.ptr as unknown as number),
          BigInt(cloneToken.ptr as unknown as number),
          BigInt(factorToken.ptr as unknown as number),
        ],
        writeVTableBytes
      );

      const handle = exports.boltffi_init_class_rn_poc_counter_new(10);
      let callbackHandleId: bigint;
      try {
        callbackHandleId = registry.register({ factor: () => 6 });
        const vtablePtr = bootstrapCallbackVTable(
          exports as unknown as Record<string, unknown>,
          "boltffi_register_callback_rn_poc_multiplier",
          [],
          (): Pointer => {
            throw new Error("must not rebuild an already-registered vtable");
          }
        );

        const status = exports.boltffi_method_class_rn_poc_counter_set_multiplier(
          handle,
          callbackHandleId,
          vtablePtr
        );
        expect(status).toBe(NativeFfiStatus.Ok);
        expect(exports.boltffi_method_class_rn_poc_counter_scaled(handle)).toBe(60);
      } finally {
        exports.boltffi_release_class_rn_poc_counter(handle);
      }

      // Rust's Box<dyn Multiplier> drop called the vtable's `free` slot during release above,
      // which routed through `registry.release` -- the registry entry should be gone.
      expect(() => registry.lookup(callbackHandleId)).toThrow();
    } finally {
      tokens.forEach((t) => t.close());
    }
  });
});
