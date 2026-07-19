import { describe, expect, it } from "vitest";
import { AsyncFutureManager, BoltFFIModule, instantiateBoltFFI } from "../src/module.js";
import { WireReader, WireWriter, wireErr, wireOk } from "../src/wire.js";

// A hand-assembled minimal valid WASM module (no toolchain dependency) exporting
// just enough for `instantiateBoltFFI` to succeed: a 1-page memory,
// `boltffi_wasm_abi_version() -> i32` (returns 7), and
// `boltffi_wasm_return_slot_addr() -> i32` (returns 0). Built once and reused —
// see the byte-by-byte derivation this was generated from in the boltffi-fork
// track notes if it ever needs regenerating.
const MINIMAL_BOLTFFI_WASM = new Uint8Array([
  0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
  0x03, 0x02, 0x00, 0x00, 0x05, 0x03, 0x01, 0x00, 0x01, 0x07, 0x45, 0x03, 0x06, 0x6d, 0x65, 0x6d,
  0x6f, 0x72, 0x79, 0x02, 0x00, 0x18, 0x62, 0x6f, 0x6c, 0x74, 0x66, 0x66, 0x69, 0x5f, 0x77, 0x61,
  0x73, 0x6d, 0x5f, 0x61, 0x62, 0x69, 0x5f, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f, 0x6e, 0x00, 0x00,
  0x1d, 0x62, 0x6f, 0x6c, 0x74, 0x66, 0x66, 0x69, 0x5f, 0x77, 0x61, 0x73, 0x6d, 0x5f, 0x72, 0x65,
  0x74, 0x75, 0x72, 0x6e, 0x5f, 0x73, 0x6c, 0x6f, 0x74, 0x5f, 0x61, 0x64, 0x64, 0x72, 0x00, 0x01,
  0x0a, 0x0b, 0x02, 0x04, 0x00, 0x41, 0x07, 0x0b, 0x04, 0x00, 0x41, 0x00, 0x0b,
]);

type ExportFunction = (...args: number[]) => number | void;

interface RuntimeHarness {
  module: BoltFFIModule;
  freedAllocations: Array<[number, number]>;
  /** Bytes `boltffi_last_error_message` observed at its `outPtr` argument before
   * writing to it — lets a test prove the caller zeroed uninitialized scratch
   * memory first (real boltffi_core panics if it doesn't, see takeLastErrorMessage). */
  lastErrorMessageSawAtOutPtr: number[];
  freedStrings: number[];
  /** Set to configure what the next `boltffi_last_error_message` call reports. */
  nextLastErrorMessage: string;
}

function createHarness(): RuntimeHarness {
  const wasmMemory = new WebAssembly.Memory({ initial: 1 });
  const freedAllocations: Array<[number, number]> = [];
  const allocations = new Map<number, number>();
  const returnSlotAddress = 16;
  let nextPointer = 256;
  const lastErrorMessageSawAtOutPtr: number[] = [];
  const freedStrings: number[] = [];
  const harnessState = { nextLastErrorMessage: "" };

  const exports: Record<string, ExportFunction | WebAssembly.Memory> = {
    memory: wasmMemory,
    boltffi_wasm_abi_version: () => 1,
    boltffi_wasm_alloc: (size: number) => {
      if (size === 0) {
        return 0;
      }
      const pointer = nextPointer;
      nextPointer += size;
      allocations.set(pointer, size);
      return pointer;
    },
    boltffi_wasm_free: (ptr: number, size: number) => {
      if (ptr === 0 || size === 0) {
        return;
      }
      freedAllocations.push([ptr, size]);
      allocations.delete(ptr);
    },
    boltffi_wasm_free_buf: (ptr: number, size: number) => {
      if (ptr === 0 || size === 0) {
        return;
      }
      freedAllocations.push([ptr, size]);
      allocations.delete(ptr);
    },
    boltffi_wasm_realloc: (ptr: number, oldSize: number, newSize: number) => {
      if (newSize === 0) {
        if (ptr !== 0 && oldSize !== 0) {
          freedAllocations.push([ptr, oldSize]);
          allocations.delete(ptr);
        }
        return 0;
      }
      if (ptr === 0) {
        const pointer = nextPointer;
        nextPointer += newSize;
        allocations.set(pointer, newSize);
        return pointer;
      }
      const pointer = nextPointer;
      nextPointer += newSize;
      const memoryBytes = new Uint8Array(wasmMemory.buffer);
      const copyLength = Math.min(oldSize, newSize);
      memoryBytes.set(memoryBytes.subarray(ptr, ptr + copyLength), pointer);
      allocations.delete(ptr);
      allocations.set(pointer, newSize);
      return pointer;
    },
    boltffi_wasm_free_string_return: (ptr: number, len: number) => {
      if (ptr === 0 || len === 0) {
        return;
      }
      freedAllocations.push([ptr, len]);
      allocations.delete(ptr);
    },
    boltffi_wasm_return_slot_addr: () => returnSlotAddress,
    // Mirrors real boltffi_core::boltffi_last_error_message: writes an FfiString
    // (ptr: u32, len: u32, cap: u32) at `outPtr`. Records the bytes it found
    // there first, the way `*out = FfiString::from(message)` would implicitly
    // drop whatever FfiString was already there before overwriting it.
    boltffi_last_error_message: (outPtr: number) => {
      lastErrorMessageSawAtOutPtr.push(
        ...Array.from(new Uint8Array(wasmMemory.buffer, outPtr, 12))
      );
      const bytes = new TextEncoder().encode(harnessState.nextLastErrorMessage);
      const stringPointer = nextPointer;
      nextPointer += bytes.length;
      new Uint8Array(wasmMemory.buffer, stringPointer, bytes.length).set(bytes);
      const view = new DataView(wasmMemory.buffer);
      view.setUint32(outPtr, bytes.length === 0 ? 0 : stringPointer, true);
      view.setUint32(outPtr + 4, bytes.length, true);
      view.setUint32(outPtr + 8, bytes.length, true);
      return 0;
    },
    boltffi_free_string: (ptr: number) => {
      freedStrings.push(ptr);
    },
  };

  const instance = { exports } as unknown as WebAssembly.Instance;
  const asyncManager = new AsyncFutureManager();
  return {
    module: new BoltFFIModule(instance, asyncManager),
    freedAllocations,
    lastErrorMessageSawAtOutPtr,
    freedStrings,
    get nextLastErrorMessage(): string {
      return harnessState.nextLastErrorMessage;
    },
    set nextLastErrorMessage(value: string) {
      harnessState.nextLastErrorMessage = value;
    },
  };
}

describe("WireReader and WireWriter", () => {
  it("round-trips primitive, string, and bytes values", () => {
    const writer = new WireWriter();
    writer.writeBool(true);
    writer.writeI8(-5);
    writer.writeU8(250);
    writer.writeI16(-1234);
    writer.writeU16(54321);
    writer.writeI32(-123_456_789);
    writer.writeU32(3_456_789_012);
    writer.writeI64(-9_000_000_000n);
    writer.writeU64(18_000_000_000n);
    writer.writeISize(-17n);
    writer.writeUSize(23n);
    writer.writeF32(Math.PI);
    writer.writeF64(Math.E);
    writer.writeString("boltffi");
    writer.writeBytes(Uint8Array.from([1, 2, 3, 4, 5]));

    const reader = new WireReader(writer.getBytes().buffer);
    expect(reader.readBool()).toBe(true);
    expect(reader.readI8()).toBe(-5);
    expect(reader.readU8()).toBe(250);
    expect(reader.readI16()).toBe(-1234);
    expect(reader.readU16()).toBe(54321);
    expect(reader.readI32()).toBe(-123_456_789);
    expect(reader.readU32()).toBe(3_456_789_012);
    expect(reader.readI64()).toBe(-9_000_000_000n);
    expect(reader.readU64()).toBe(18_000_000_000n);
    expect(reader.readISize()).toBe(-17n);
    expect(reader.readUSize()).toBe(23n);
    expect(reader.readF32()).toBeCloseTo(Math.PI, 5);
    expect(reader.readF64()).toBeCloseTo(Math.E, 12);
    expect(reader.readString()).toBe("boltffi");
    expect(Array.from(reader.readBytes())).toEqual([1, 2, 3, 4, 5]);
  });

  it("encodes and decodes optional and array values", () => {
    const writer = new WireWriter();
    writer.writeOptional<number>(null, (value) => writer.writeI32(value));
    writer.writeOptional<number>(33, (value) => writer.writeI32(value));
    writer.writeArray<number>([4, 5, 6], (value) => writer.writeU32(value));

    const reader = new WireReader(writer.getBytes().buffer);
    expect(reader.readOptional(() => reader.readI32())).toBeNull();
    expect(reader.readOptional(() => reader.readI32())).toBe(33);
    expect(reader.readArray(() => reader.readU32())).toEqual([4, 5, 6]);
  });

  it("decodes readResult success and error branches", () => {
    const okWriter = new WireWriter();
    okWriter.writeU8(0);
    okWriter.writeI32(44);
    const okReader = new WireReader(okWriter.getBytes().buffer);
    expect(okReader.readResult(() => okReader.readI32(), () => new Error("err"))).toBe(44);

    const errWriter = new WireWriter();
    errWriter.writeU8(1);
    errWriter.writeString("boom");
    const errReader = new WireReader(errWriter.getBytes().buffer);
    expect(() =>
      errReader.readResult(
        () => 0,
        () => new Error(errReader.readString())
      )
    ).toThrow("boom");
  });

  it("returns a detached copy for readBytes", () => {
    const writer = new WireWriter();
    writer.writeBytes(Uint8Array.from([7, 8, 9]));
    const payloadBuffer = writer.getBytes().buffer;

    const firstReader = new WireReader(payloadBuffer);
    const firstRead = firstReader.readBytes();
    firstRead[0] = 99;

    const secondReader = new WireReader(payloadBuffer);
    expect(Array.from(secondReader.readBytes())).toEqual([7, 8, 9]);
  });

  it("grows local writer capacity for large payloads", () => {
    const writer = new WireWriter(1);
    const largePayload = Uint8Array.from(Array.from({ length: 300 }, (_, index) => index % 256));
    writer.writeBytes(largePayload);

    expect(writer.len).toBe(304);
    const reader = new WireReader(writer.getBytes().buffer);
    expect(Array.from(reader.readBytes())).toEqual(Array.from(largePayload));
  });
});

describe("WireWriter result encoding", () => {
  it("encodes explicit ok and err tags deterministically", () => {
    const okWriter = new WireWriter();
    okWriter.writeResult(
      wireOk(42),
      (value) => {
        okWriter.writeI32(value);
      },
      () => {
        throw new Error("unexpected err branch");
      }
    );
    const okReader = new WireReader(okWriter.getBytes().buffer);
    expect(okReader.readU8()).toBe(0);
    expect(okReader.readI32()).toBe(42);

    const errWriter = new WireWriter();
    errWriter.writeResult(
      wireErr({ code: 7 }),
      () => {
        throw new Error("unexpected ok branch");
      },
      (error) => {
        errWriter.writeU32(error.code);
      }
    );
    const errReader = new WireReader(errWriter.getBytes().buffer);
    expect(errReader.readU8()).toBe(1);
    expect(errReader.readU32()).toBe(7);
  });

  it("uses Error objects as err branch fallback", () => {
    const writer = new WireWriter();
    writer.writeResult<number, Error>(
      new Error("fallback"),
      () => {
        throw new Error("unexpected ok branch");
      },
      (error) => {
        writer.writeString(error.message);
      }
    );

    const reader = new WireReader(writer.getBytes().buffer);
    expect(reader.readU8()).toBe(1);
    expect(reader.readString()).toBe("fallback");
  });

  it("rejects ambiguous object payloads without explicit result tagging", () => {
    const writer = new WireWriter();
    expect(() =>
      writer.writeResult(
        { code: 7 } as unknown as number,
        () => {
          throw new Error("unexpected ok branch");
        },
        () => {
          throw new Error("unexpected err branch");
        }
      )
    ).toThrow("Ambiguous Result object");
  });
});

describe("BoltFFIModule memory operations", () => {
  it("allocString writes UTF-8 bytes and freeAlloc releases them", () => {
    const { module, freedAllocations } = createHarness();
    const allocation = module.allocString("hello");
    expect(allocation.ptr).toBeGreaterThan(0);
    expect(allocation.len).toBe(5);
    expect(Array.from(module.readFromMemory(allocation.ptr, allocation.len))).toEqual([
      104, 101, 108, 108, 111,
    ]);

    module.freeAlloc(allocation);
    expect(freedAllocations).toContainEqual([allocation.ptr, allocation.len]);
  });

  it("handles empty string allocations without invalid free", () => {
    const { module, freedAllocations } = createHarness();
    const allocation = module.allocString("");
    expect(allocation.ptr).toBe(0);
    expect(allocation.len).toBe(0);
    module.freeAlloc(allocation);
    expect(freedAllocations).toEqual([]);
  });

  it("reads and frees result buffers through descriptor pointer", () => {
    const { module, freedAllocations } = createHarness();
    const payloadPointer = 1024;
    const payloadCapacity = 32;
    const encodedPayloadWriter = new WireWriter();
    encodedPayloadWriter.writeBytes(Uint8Array.from([10, 11, 12, 13]));
    const encodedPayload = encodedPayloadWriter.getBytes();
    module.writeToMemory(payloadPointer, encodedPayload);

    const descriptorPointer = 2048;
    const descriptorView = new DataView(new ArrayBuffer(16));
    descriptorView.setUint32(0, payloadPointer, true);
    descriptorView.setUint32(4, encodedPayload.length, true);
    descriptorView.setUint32(8, payloadCapacity, true);
    descriptorView.setUint32(12, 1, true);
    module.writeToMemory(descriptorPointer, new Uint8Array(descriptorView.buffer));

    const reader = module.readerFromBuf(descriptorPointer);
    expect(Array.from(reader.readBytes())).toEqual([10, 11, 12, 13]);

    module.freeBuf(descriptorPointer);
    expect(freedAllocations).toContainEqual([payloadPointer, payloadCapacity]);
    expect(freedAllocations).toContainEqual([descriptorPointer, 16]);
  });

  it("freeBufDescriptor releases descriptor allocation only", () => {
    const { module, freedAllocations } = createHarness();
    const descriptorPointer = 4096;
    module.freeBufDescriptor(descriptorPointer);
    expect(freedAllocations).toContainEqual([descriptorPointer, 16]);
  });

  it("allocWriter reallocates when payload outgrows initial capacity", () => {
    const { module, freedAllocations } = createHarness();
    const writer = module.allocWriter(1);
    const initialPointer = writer.ptr;
    const payload = Uint8Array.from(Array.from({ length: 400 }, (_, index) => index % 256));

    writer.writeBytes(payload);

    expect(writer.ptr).toBeGreaterThan(0);
    expect(writer.ptr).not.toBe(initialPointer);
    expect(writer.capacity).toBeGreaterThan(64);
    expect(Array.from(module.readFromMemory(writer.ptr, writer.len))).toEqual([
      144, 1, 0, 0,
      ...Array.from(payload),
    ]);

    const pointer = writer.ptr;
    const capacity = writer.capacity;
    module.freeWriter(writer);
    expect(freedAllocations).not.toContainEqual([pointer, capacity]);
  });

  it("allocPrimitiveBuffer writes i32 elements and frees by byte size", () => {
    const { module, freedAllocations } = createHarness();
    const allocation = module.allocPrimitiveBuffer([1, -2, 3, -4], "i32");

    expect(allocation.len).toBe(4);
    expect(allocation.allocationSize).toBe(16);

    const raw = module.readFromMemory(allocation.ptr, allocation.allocationSize);
    const view = new DataView(raw.buffer, raw.byteOffset, raw.byteLength);
    expect(view.getInt32(0, true)).toBe(1);
    expect(view.getInt32(4, true)).toBe(-2);
    expect(view.getInt32(8, true)).toBe(3);
    expect(view.getInt32(12, true)).toBe(-4);

    module.freePrimitiveBuffer(allocation);
    expect(freedAllocations).toContainEqual([allocation.ptr, allocation.allocationSize]);
  });

  it("reuses pooled writers when capacity matches", () => {
    const { module } = createHarness();
    const writer = module.allocWriter(8);
    const pointer = writer.ptr;
    writer.writeU32(42);
    module.freeWriter(writer);

    const reusedWriter = module.allocWriter(8);
    expect(reusedWriter.ptr).toBe(pointer);
    expect(reusedWriter.len).toBe(0);
  });

  it("takeLastErrorMessage zeroes uninitialized scratch memory before reading the native write", () => {
    // boltffi_wasm_alloc returns uninitialized memory in the real wasm build.
    // boltffi_last_error_message writes its result via `*out =
    // FfiString::from(message)`, which drops whatever FfiString was already at
    // `out` first — non-zero garbage there reads as a bogus non-null
    // pointer/capacity and crashes the real allocator (confirmed against a real
    // compiled demo.wasm). Poison the slot the same way uninitialized memory
    // would look, and prove takeLastErrorMessage zeroes it before the native
    // call — this is the regression the real end-to-end run caught.
    const harness = createHarness();
    const { module, freedAllocations, freedStrings, lastErrorMessageSawAtOutPtr } = harness;
    harness.nextLastErrorMessage = "boom";
    module.writeToMemory(256, new Uint8Array(12).fill(0xff));

    const message = module.takeLastErrorMessage();

    expect(message).toBe("boom");
    expect(lastErrorMessageSawAtOutPtr).toEqual(new Array(12).fill(0));
    expect(freedStrings).toEqual([256]);
    expect(freedAllocations).toContainEqual([256, 12]);
  });

  it("takeLastErrorMessage returns an empty string when no error was set", () => {
    const { module } = createHarness();
    expect(module.takeLastErrorMessage()).toBe("");
  });
});

describe("BoltFFIModule async completion", () => {
  it("throws invalid argument from completion status", () => {
    const { module, freedAllocations } = createHarness();

    expect(() =>
      module.completeAsync((statusPtr) => {
        const status = new DataView(new ArrayBuffer(4));
        status.setInt32(0, 3, true);
        module.writeToMemory(statusPtr, new Uint8Array(status.buffer));
        return 0;
      })
    ).toThrow("invalid argument");

    expect(freedAllocations).toContainEqual([256, 4]);
  });
});

describe("instantiateBoltFFI", () => {
  it("accepts an ArrayBuffer source", async () => {
    const mod = await instantiateBoltFFI(MINIMAL_BOLTFFI_WASM.buffer, 7);
    expect(mod.exports.boltffi_wasm_abi_version()).toBe(7);
  });

  it("accepts a Response source", async () => {
    const response = new Response(MINIMAL_BOLTFFI_WASM);
    const mod = await instantiateBoltFFI(response, 7);
    expect(mod.exports.boltffi_wasm_abi_version()).toBe(7);
  });

  /// Regression test for a bug found in the Cloudflare Workers harness:
  /// bundlers' ES-module `.wasm` import (`import wasmModule from "./foo.wasm"`)
  /// hands the caller an already-compiled `WebAssembly.Module`, not raw bytes —
  /// Workers has no filesystem and no other sanctioned way to get a `.wasm`
  /// asset's bytes. `WebAssembly.instantiate(module, importObject)` resolves
  /// straight to an `Instance` per spec (only the BufferSource/Response overload
  /// resolves `{module, instance}`), so the old code's unconditional
  /// `const { instance } = await WebAssembly.instantiate(source, ...)` destructured
  /// `undefined` out of the resolved `Instance` and `new BoltFFIModule(undefined, ...)`
  /// threw reading `.exports`.
  it("accepts an already-compiled WebAssembly.Module source", async () => {
    const precompiled = await WebAssembly.compile(MINIMAL_BOLTFFI_WASM);
    const mod = await instantiateBoltFFI(precompiled, 7);
    expect(mod.exports.boltffi_wasm_abi_version()).toBe(7);
  });

  it("throws a clear ABI mismatch error rather than silently continuing", async () => {
    await expect(instantiateBoltFFI(MINIMAL_BOLTFFI_WASM.buffer, 99)).rejects.toThrow(
      "BoltFFI ABI version mismatch: expected 99, got 7"
    );
  });
});
