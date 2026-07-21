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
});
