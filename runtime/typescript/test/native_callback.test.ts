// TDD coverage for the react-native track's native-mode host-callback registration
// (docs/tracks/react-native.md): `native_callback.ts`'s per-trait vtable bootstrap + per-instance
// id/ref-count registry. Server-free, mock-host by design (no Rust toolchain, no Bun) -- the
// backend-specific half (turning a JS closure into a real native function pointer) is a HOST
// concern this file never touches (see `native.bun.test.ts` for the real-dylib proof of that
// half, driven through Bun's `JSCallback`).
import { describe, expect, it, beforeEach } from "vitest";
import {
  bootstrapCallbackVTable,
  NativeCallbackTraitRegistry,
  _resetCallbackVTableRegistrationsForTests,
  type NativeCallbackHostExports,
} from "../src/native_callback.js";

beforeEach(() => {
  _resetCallbackVTableRegistrationsForTests();
});

describe("bootstrapCallbackVTable", () => {
  it("writes the vtable bytes and registers exactly once", () => {
    const registerCalls: bigint[] = [];
    const exports: NativeCallbackHostExports = {
      boltffi_register_callback_demo_listener: (ptr: bigint) => registerCalls.push(ptr),
    };
    let writeCalls = 0;
    const writeVTableBytes = (fieldPtrs: readonly bigint[]): bigint => {
      writeCalls += 1;
      expect(fieldPtrs).toEqual([1n, 2n, 3n]);
      return 0xabcn;
    };

    const first = bootstrapCallbackVTable(
      exports,
      "boltffi_register_callback_demo_listener",
      [1n, 2n, 3n],
      writeVTableBytes
    );
    expect(first).toBe(0xabcn);
    expect(writeCalls).toBe(1);
    expect(registerCalls).toEqual([0xabcn]);
  });

  it("never re-registers or re-allocates a trait already bootstrapped", () => {
    const registerCalls: bigint[] = [];
    const exports: NativeCallbackHostExports = {
      boltffi_register_callback_demo_listener: (ptr: bigint) => registerCalls.push(ptr),
    };
    let writeCalls = 0;
    const writeVTableBytes = (): bigint => {
      writeCalls += 1;
      return 0x111n;
    };

    bootstrapCallbackVTable(exports, "boltffi_register_callback_demo_listener", [1n, 2n], writeVTableBytes);
    const second = bootstrapCallbackVTable(
      exports,
      "boltffi_register_callback_demo_listener",
      [1n, 2n],
      writeVTableBytes
    );

    expect(second).toBe(0x111n);
    expect(writeCalls).toBe(1); // NOT called a second time
    expect(registerCalls).toEqual([0x111n]); // NOT registered a second time
  });

  it("bootstraps two DIFFERENT traits independently", () => {
    const exports: NativeCallbackHostExports = {
      boltffi_register_callback_demo_listener: () => {},
      boltffi_register_callback_demo_multiplier: () => {},
    };
    const a = bootstrapCallbackVTable(exports, "boltffi_register_callback_demo_listener", [1n], () => 0xa1n);
    const b = bootstrapCallbackVTable(exports, "boltffi_register_callback_demo_multiplier", [2n], () => 0xb2n);
    expect(a).toBe(0xa1n);
    expect(b).toBe(0xb2n);
  });

  it("throws loudly when the register export is missing rather than silently skipping", () => {
    expect(() => bootstrapCallbackVTable({}, "boltffi_register_callback_missing", [1n], () => 0n)).toThrow(
      /boltffi_register_callback_missing/
    );
  });
});

describe("NativeCallbackTraitRegistry", () => {
  it("mints a fresh id with ref count 1 on register, and finds it via lookup", () => {
    const registry = new NativeCallbackTraitRegistry<{ factor(): number }>();
    const impl = { factor: () => 4 };
    const id = registry.register(impl);
    expect(registry.lookup(id)).toBe(impl);
  });

  it("mints DISTINCT ids for separate register() calls, even for the same impl object", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    const impl = {};
    const a = registry.register(impl);
    const b = registry.register(impl);
    expect(a).not.toBe(b);
  });

  it("retain increments the ref count without minting a new id", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    const id = registry.register({});
    expect(registry.retain(id)).toBe(id);
    // Two retains (on top of the initial register) => ref count 3; three releases needed to reach 0.
    registry.retain(id);
    expect(registry.release(id)).toBe(false);
    expect(registry.release(id)).toBe(false);
    expect(registry.release(id)).toBe(true); // the third release is the one that reaches zero
  });

  it("release removes the entry once the ref count reaches zero, and lookup then throws", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    const id = registry.register({});
    expect(registry.release(id)).toBe(true);
    expect(() => registry.lookup(id)).toThrow(/not found/);
  });

  it("retain on an unknown handle throws rather than silently minting bookkeeping", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    expect(() => registry.retain(999n)).toThrow(/unknown callback handle/);
  });

  it("release on an unknown handle throws rather than silently no-op'ing", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    expect(() => registry.release(999n)).toThrow(/unknown callback handle/);
  });

  it("lookup on a never-registered handle throws", () => {
    const registry = new NativeCallbackTraitRegistry<object>();
    expect(() => registry.lookup(1n)).toThrow(/not found/);
  });
});
