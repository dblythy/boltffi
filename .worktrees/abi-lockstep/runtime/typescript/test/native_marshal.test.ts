// TDD coverage for the react-native track's stage-4 buffer marshaling
// (docs/tracks/react-native.md): `NativeBoltFFIModule`'s reimplementation of `BoltFFIModule`'s
// full alloc/take/packed/scratch surface against a pure-JS `NativeMemoryArena`. Unlike
// `native.bun.test.ts` (real dylib, Bun-only, scalar-only Counter/Multiplier fixture), this suite
// runs under plain vitest/Node — no Rust toolchain, no Bun — and exercises exactly the surface
// that was previously entirely missing: string/bytes/primitive-array/record allocation and every
// buffer-encoded return route (packed bigint, buf descriptor, return slot, last-error-message).
import { describe, expect, it } from "vitest";
import {
  NativeAsyncFutureManager,
  NativeBoltFFIModule,
  NativeContinuationSignal,
  instantiateBoltFFINative,
} from "../src/native.js";
import { NativeMemoryArena, RETURN_SLOT_SIZE } from "../src/native_arena.js";
import { WireReader } from "../src/wire.js";

function makeModule(): NativeBoltFFIModule<Record<string, unknown>> {
  return new NativeBoltFFIModule({}, () => ({}));
}

describe("NativeMemoryArena", () => {
  it("allocates non-overlapping regions and never hands out the reserved return-slot header", () => {
    const arena = new NativeMemoryArena(64);
    const a = arena.alloc(8);
    const b = arena.alloc(8);
    expect(a).toBeGreaterThanOrEqual(RETURN_SLOT_SIZE);
    expect(b).toBeGreaterThanOrEqual(a + 8);
  });

  it("grows the backing buffer (and copies existing bytes) when capacity is exceeded", () => {
    const arena = new NativeMemoryArena(32);
    const ptr = arena.alloc(8);
    arena.byteView.set([1, 2, 3, 4, 5, 6, 7, 8], ptr);
    const before = arena.buffer;

    const big = arena.alloc(1024);
    expect(arena.buffer).not.toBe(before);
    expect(arena.buffer.byteLength).toBeGreaterThanOrEqual(1024 + big);
    expect(Array.from(arena.byteView.slice(ptr, ptr + 8))).toEqual([1, 2, 3, 4, 5, 6, 7, 8]);
  });

  it("fires the onGrow hook with the new buffer identity", () => {
    const arena = new NativeMemoryArena(16);
    const seen: ArrayBuffer[] = [];
    arena.setOnGrow((buf) => seen.push(buf));
    arena.alloc(1024);
    expect(seen).toHaveLength(1);
    expect(seen[0]).toBe(arena.buffer);
  });

  it("reuses freed blocks (first-fit) instead of always bumping the high-water mark", () => {
    const arena = new NativeMemoryArena(4096);
    const a = arena.alloc(16);
    arena.free(a, 16);
    const b = arena.alloc(16);
    expect(b).toBe(a);
  });

  it("realloc grows in place logically (copies old bytes, frees the old block)", () => {
    const arena = new NativeMemoryArena(4096);
    const ptr = arena.alloc(4);
    arena.byteView.set([9, 9, 9, 9], ptr);
    const grown = arena.realloc(ptr, 4, 16);
    expect(Array.from(arena.byteView.slice(grown, grown + 4))).toEqual([9, 9, 9, 9]);
  });

  it("realloc is a no-op when shrinking or same-sizing", () => {
    const arena = new NativeMemoryArena(4096);
    const ptr = arena.alloc(16);
    expect(arena.realloc(ptr, 16, 8)).toBe(ptr);
    expect(arena.realloc(ptr, 16, 16)).toBe(ptr);
  });

  it("regression: every allocation is 8-byte aligned, even right after an odd-length allocation (Codex finding 2, HIGH alignment -- the native side reads i64/f64/descriptor slots as typed pointers, which is UB when misaligned)", () => {
    const arena = new NativeMemoryArena();
    arena.alloc(1); // odd-length string-like allocation, mimicking allocString("x")
    const followUp = arena.alloc(8); // an i64/f64 array or a 16-byte buf descriptor slot
    expect(followUp % 8).toBe(0);
  });

  it("regression: alignment survives a free/reuse cycle through the first-fit free list", () => {
    const arena = new NativeMemoryArena();
    arena.alloc(3);
    const a = arena.alloc(5); // deliberately odd-sized so a naive splitter could misalign the remainder
    arena.free(a, 5);
    const reused = arena.alloc(8);
    expect(reused % 8).toBe(0);
  });

  // Second adversarial round, finding 2 (HIGH): pinning the arena's backing buffer for a call's
  // duration (runtime/cpp's `boltffi_generic_host_object.cpp`) preserves the buffer's LIFETIME,
  // not the COHERENCE of a write through an out-param pointer computed before a reentrant grow --
  // concretely, `subscribe_with_listener(..., int32_t* return_out)` synchronously invokes
  // `transport.send` (parse-core-rs's `livequery.rs`), which can allocate JS-side and grow this
  // arena, replacing the buffer BEFORE Rust writes the request id through the pointer it was
  // already handed (into the OLD buffer) -- JS then reads the offset out of the NEW buffer and
  // sees garbage, not the value Rust wrote. `beginCall`/`endCall` close that gap by making
  // `grow()` refuse to run at all while ANY call is in flight, so a reentrant allocation that
  // would need more room fails LOUDLY instead of silently corrupting an in-flight call's already-
  // translated addresses. The call-boundary marking itself belongs at the native-call dispatch
  // site (native.ts's `NativeBoltFFIModule`/`NativeAsyncFutureManager`, wired below in this same
  // file's "call-in-flight growth guard wiring" suite; the equivalent point in the C++ HostObject
  // deliberately has no mirror, see that class's header doc) -- this describe block proves the
  // enforced PRIMITIVE in isolation.
  describe("NativeMemoryArena: call-in-flight growth guard (reentrant-growth coherence)", () => {
    it("refuses to grow while a call is marked in-flight, instead of silently reallocating", () => {
      const arena = new NativeMemoryArena(32);
      arena.beginCall();
      try {
        expect(() => arena.alloc(1024)).toThrow(/in flight/i);
      } finally {
        arena.endCall();
      }
    });

    it("an alloc that fits within existing capacity still succeeds during an active call -- only GROWTH is forbidden", () => {
      const arena = new NativeMemoryArena(4096);
      arena.beginCall();
      try {
        expect(arena.alloc(8)).toBeGreaterThanOrEqual(0);
      } finally {
        arena.endCall();
      }
    });

    it("allows growth again once the call has ended", () => {
      const arena = new NativeMemoryArena(32);
      arena.beginCall();
      arena.endCall();
      expect(() => arena.alloc(1024)).not.toThrow();
    });

    it("nests correctly: growth stays forbidden until the OUTERMOST call ends (a reentrant call nested inside an outer one must not re-enable growth early)", () => {
      const arena = new NativeMemoryArena(32);
      arena.beginCall(); // outer call begins
      arena.beginCall(); // a reentrant call nested inside it begins
      arena.endCall(); // the reentrant call returns
      expect(() => arena.alloc(1024)).toThrow(/in flight/i); // outer call is still active
      arena.endCall(); // outer call returns
      expect(() => arena.alloc(1024)).not.toThrow();
    });

    it("endCall() without a matching beginCall() throws loudly rather than going silently negative", () => {
      const arena = new NativeMemoryArena(32);
      expect(() => arena.endCall()).toThrow();
    });
  });

  // Wiring the primitive above around the actual native-call dispatch sites (native.ts):
  // `NativeAsyncFutureManager.dispatchPoll` (initial poll registration + every MaybeReady re-poll),
  // `NativeBoltFFIModule.completeAsync`, and `takeLastErrorMessage`'s two calls. These tests prove
  // the wiring actually catches the concrete scenario the second adversarial round's finding 2
  // named: an out-param native call whose OWN synchronous host-callback reenters JS and allocates
  // enough to force growth mid-call (`subscribe_with_listener`'s `transport.send` is the real
  // example; these tests reproduce the shape generically since this suite has no real dylib).
  describe("NativeBoltFFIModule/NativeAsyncFutureManager: call-in-flight growth guard wiring", () => {
    it("regression: completeAsync brackets its native dispatch -- a reentrant callback that forces arena growth mid-call (the out-param call + reentrant allocating callback scenario) throws loudly instead of silently corrupting the pending status write", () => {
      const arena = new NativeMemoryArena(32);
      const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
      expect(() =>
        module_.completeAsync((statusPtr) => {
          void statusPtr;
          // Mirrors a synchronous host callback (e.g. LiveQueryTransport::send) reentering JS and
          // allocating enough to force a grow while `complete`'s own out-param write is pending.
          arena.alloc(1024);
          return undefined;
        })
      ).toThrow(/in flight/i);
    });

    it("regression: takeLastErrorMessage brackets both of its native dispatches -- a reentrant allocating host callback forcing growth while the ptr/len out-param write is pending throws loudly", () => {
      const arena = new NativeMemoryArena(32);
      const exports = {
        boltffi_last_error_message: (outPtr: number) => {
          void outPtr;
          arena.alloc(1024); // reentrant growth attempt mid-call, before the out-param is written
        },
        boltffi_free_string: () => {},
      };
      const module_ = new NativeBoltFFIModule(exports, () => ({}), arena);
      expect(() => module_.takeLastErrorMessage()).toThrow(/in flight/i);
    });

    it("regression: pollAsyncNative brackets its initial poll dispatch -- a reentrant allocating callback forcing growth mid-registration rejects the returned promise loudly rather than corrupting it", async () => {
      const arena = new NativeMemoryArena(32);
      const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
      const poll = () => {
        arena.alloc(1024); // a synchronous host callback the registration call triggers
      };
      await expect(module_.asyncManager.pollAsyncNative(1n, poll)).rejects.toThrow(/in flight/i);
    });

    it("nested native dispatches (e.g. a synchronous host callback that itself registers another async poll before the outer call returns) keep growth forbidden until the OUTERMOST call ends", () => {
      const arena = new NativeMemoryArena(32); // small on purpose: 1024 bytes always needs a grow
      const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
      let innerRan = false;

      const outerResult = module_.completeAsync((statusPtr) => {
        void statusPtr;
        // A reentrant nested dispatch through a DIFFERENT native.ts funnel -- the Promise
        // executor (and thus `poll`) runs synchronously, so this proves nesting across funnels,
        // not just within one.
        void module_.asyncManager.pollAsyncNative(2n, () => {
          innerRan = true;
          expect(() => arena.alloc(1024)).toThrow(/in flight/i); // still forbidden: outer call active
        });
        return 42;
      });

      expect(innerRan).toBe(true);
      expect(outerResult).toBe(42);
      expect(() => arena.alloc(1024)).not.toThrow(); // legal again: both calls have returned
    });

    it("growth between an async poll's initial dispatch and its later completion (the await gap) is legal -- only the synchronous native-call window itself is protected", async () => {
      const arena = new NativeMemoryArena(32);
      let onSignal: ((callbackData: bigint, signal: NativeContinuationSignal) => void) | null = null;
      const manager = new NativeAsyncFutureManager<null>((signal) => {
        onSignal = signal;
        return null;
      }, arena);

      const poll = () => {
        // Registration only -- no growth attempted inside the call itself.
      };
      const promise = manager.pollAsyncNative(1n, poll);

      // We're now in the await gap: the synchronous dispatch already returned, nothing is
      // in flight, so growth here must be allowed.
      expect(() => arena.alloc(1024)).not.toThrow();

      onSignal!(1n, NativeContinuationSignal.Ready);
      await expect(promise).resolves.toBe(1n);
    });

    it("regression: beginCall/endCall stay balanced across repeated queued MaybeReady re-polls -- each re-poll gets its OWN bracket, never one held open across the microtask hop (no depth leak, no cross-poll nesting)", async () => {
      let onSignal: ((callbackData: bigint, signal: NativeContinuationSignal) => void) | null = null;
      let depth = 0;
      let maxDepthSeen = 0;
      const callMarks = {
        beginCall: () => {
          depth += 1;
          maxDepthSeen = Math.max(maxDepthSeen, depth);
        },
        endCall: () => {
          depth -= 1;
        },
      };
      const manager = new NativeAsyncFutureManager<null>((signal) => {
        onSignal = signal;
        return null;
      }, callMarks);

      const TOTAL_ITERATIONS = 1000;
      let pollCount = 0;
      const poll = (_handle: bigint, callbackData: bigint) => {
        pollCount += 1;
        if (pollCount >= TOTAL_ITERATIONS) {
          onSignal!(callbackData, NativeContinuationSignal.Ready);
        } else {
          onSignal!(callbackData, NativeContinuationSignal.MaybeReady);
        }
      };

      const result = await manager.pollAsyncNative(1n, poll);
      expect(result).toBe(1n);
      expect(depth).toBe(0);
      expect(maxDepthSeen).toBe(1);
    });
  });
});

describe("NativeBoltFFIModule: string/bytes/primitive-array params", () => {
  it("allocString round-trips UTF-8 bytes readable via readFromMemory", () => {
    const module_ = makeModule();
    const alloc = module_.allocString("héllo 🌍");
    const roundTripped = new TextDecoder().decode(module_.readFromMemory(alloc.ptr, alloc.len));
    expect(roundTripped).toBe("héllo 🌍");
    expect(() => module_.freeAlloc(alloc)).not.toThrow();
  });

  it("allocString handles the empty string", () => {
    const module_ = makeModule();
    const alloc = module_.allocString("");
    expect(alloc.len).toBe(0);
    module_.freeAlloc(alloc);
  });

  it("allocBytes round-trips raw bytes", () => {
    const module_ = makeModule();
    const bytes = new Uint8Array([10, 20, 30, 255]);
    const alloc = module_.allocBytes(bytes);
    expect(Array.from(module_.readFromMemory(alloc.ptr, alloc.len))).toEqual([10, 20, 30, 255]);
    module_.freeAlloc(alloc);
  });

  it.each([
    ["allocI32Array", [1, -2, 3]] as const,
    ["allocU32Array", [1, 2, 3]] as const,
    ["allocI16Array", [1, -2, 3]] as const,
    ["allocF64Array", [1.5, -2.25]] as const,
  ])("%s allocates a decodable primitive buffer", (method, values) => {
    const module_ = makeModule();
    const alloc = (module_[method as "allocI32Array"] as (v: readonly number[]) => { ptr: number; len: number; allocationSize: number })(
      values as readonly number[]
    );
    expect(alloc.len).toBe(values.length);
    module_.freePrimitiveBuffer(alloc);
  });

  it("allocPrimitiveBuffer dispatches by element type and round-trips through writeBufDescriptor/takeBufI32Array", () => {
    const module_ = makeModule();
    const alloc = module_.allocPrimitiveBuffer([10, 20, 30], "i32");
    const descriptor = module_.allocBufDescriptor();
    module_.writeBufDescriptor(descriptor, alloc.ptr, alloc.allocationSize, alloc.allocationSize);
    const result = module_.takeBufI32Array(descriptor);
    expect(Array.from(result)).toEqual([10, 20, 30]);
  });

  it("allocBoolArray packs booleans as single bytes", () => {
    const module_ = makeModule();
    const alloc = module_.allocBoolArray([true, false, true]);
    expect(Array.from(module_.readFromMemory(alloc.ptr, alloc.len))).toEqual([1, 0, 1]);
    module_.freePrimitiveBuffer(alloc);
  });
});

describe("NativeBoltFFIModule: WireWriter-backed params", () => {
  it("allocWriter produces a WireWriter whose .ptr/.len are usable as real arena coordinates", () => {
    const module_ = makeModule();
    const writer = module_.allocWriter(16);
    writer.writeU32(0xdeadbeef);
    writer.writeString("hi");
    expect(writer.ptr).toBeGreaterThan(0);
    const bytes = module_.readFromMemory(writer.ptr, writer.len);
    expect(bytes.length).toBe(writer.len);
    module_.freeWriter(writer);
  });

  it("allocCompositeBuffer writes each element via the provided encoder", () => {
    const module_ = makeModule();
    const writer = module_.allocCompositeBuffer([1, 2, 3], 4, (w, v) => w.writeI32(v));
    expect(writer.len).toBe(12);
    module_.freeWriter(writer);
  });

  it("a writer's bytes survive arena growth mid-write (no torn reads)", () => {
    const module_ = new NativeBoltFFIModule({}, () => ({}), new NativeMemoryArena(16));
    const writer = module_.allocWriter(4);
    for (let i = 0; i < 100; i++) writer.writeU32(i);
    const view = new DataView(module_.readFromMemory(writer.ptr, writer.len).buffer);
    expect(view.getUint32(0, true)).toBe(0);
    expect(view.getUint32(4 * 99, true)).toBe(99);
  });
});

describe("NativeBoltFFIModule: buffer descriptors + packed returns", () => {
  it("allocBufDescriptor/writeBufDescriptor/readerFromBuf/freeBuf round-trip an encoded WireReader", () => {
    const module_ = makeModule();
    const writer = module_.allocWriter(8);
    writer.writeU32(42);
    const descriptor = module_.allocBufDescriptor();
    module_.writeBufDescriptor(descriptor, writer.ptr, writer.len, writer.capacity);
    const reader = module_.readerFromBuf(descriptor);
    expect(reader.readU32()).toBe(42);
    expect(() => module_.freeBuf(descriptor)).not.toThrow();
  });

  it("regression: takeBufI32Array must not free the payload itself -- the generated finally's freeBuf owns that, or two unrelated later allocations alias the same pointer (Codex finding 1, HIGH double-free)", () => {
    const module_ = makeModule();
    const alloc = module_.allocPrimitiveBuffer([1, 2, 3], "i32"); // 12 bytes
    const descriptor = module_.allocBufDescriptor();
    module_.writeBufDescriptor(descriptor, alloc.ptr, alloc.allocationSize, alloc.allocationSize);

    // Mirrors class.txt/async_function.txt's generated async packed-return route exactly:
    // `const reader = _module.readerFromBuf(outPtr); const result = <decode_expr>;` followed by
    // the surrounding `finally { if (completeCompleted) _module.freeBuf(outPtr); }`. For a
    // Vec<i32> return, decode_expr is literally `_module.takeBufI32Array(outPtr)` (lower.rs's
    // direct_vec_output_route buf_decode table) -- so both calls below fire on the SAME
    // descriptor in every generated caller, not just in this test.
    const decoded = module_.takeBufI32Array(descriptor);
    expect(Array.from(decoded)).toEqual([1, 2, 3]);
    module_.freeBuf(descriptor);

    // If the payload pointer was freed twice, the free-list holds two identical (ptr, len)
    // entries (coalescing only merges *adjacent* blocks, not identical/duplicate ones), so two
    // unrelated 12-byte allocations right after would both be handed the same stale pointer --
    // silent aliasing between two live buffers.
    const other1 = module_.allocU8Array(new Uint8Array(12));
    const other2 = module_.allocU8Array(new Uint8Array(12));
    expect(other1.ptr).not.toBe(other2.ptr);
  });

  it("takePackedUtf8String decodes and frees a manually packed string allocation", () => {
    const module_ = makeModule();
    const alloc = module_.allocString("packed value");
    const packed = (BigInt(alloc.len) << 32n) | BigInt(alloc.ptr >>> 0);
    expect(module_.takePackedUtf8String(packed)).toBe("packed value");
  });

  it("takePackedUtf8String returns empty string for a null/zero-length packed value", () => {
    const module_ = makeModule();
    expect(module_.takePackedUtf8String(0n)).toBe("");
  });

  it("takePackedBuffer decodes a manually packed WireWriter payload", () => {
    const module_ = makeModule();
    const writer = module_.allocWriter(8);
    writer.writeU32(7);
    writer.writeU32(8);
    const packed = (BigInt(writer.len) << 32n) | BigInt(writer.ptr >>> 0);
    const reader = module_.takePackedBuffer(packed);
    expect(reader.readU32()).toBe(7);
    expect(reader.readU32()).toBe(8);
  });

  it.each([
    ["takePackedI32Array", "allocI32Array", [1, -2, 3]] as const,
    ["takePackedU8Array", "allocU8Array", [1, 2, 3]] as const,
    ["takePackedF64Array", "allocF64Array", [1.5, 2.5]] as const,
  ])("%s decodes a manually packed %s allocation", (takeMethod, allocMethod, values) => {
    const module_ = makeModule();
    const alloc = (
      module_[allocMethod as "allocI32Array"] as (v: readonly number[]) => { ptr: number; allocationSize: number }
    )(values as readonly number[]);
    const packed = (BigInt(alloc.allocationSize) << 32n) | BigInt(alloc.ptr >>> 0);
    const result = (module_[takeMethod as "takePackedI32Array"] as (p: bigint) => ArrayLike<number>)(packed);
    expect(Array.from(result as ArrayLike<number>)).toEqual(values as unknown as number[]);
  });

  it.each([
    ["takePackedOptionalI32", 4, (v: DataView, o: number) => v.setInt32(o, -5, true), -5] as const,
    ["takePackedOptionalU8", 1, (v: DataView, o: number) => v.setUint8(o, 200), 200] as const,
    ["takePackedOptionalBool", 1, (v: DataView, o: number) => v.setUint8(o, 1), true] as const,
  ])("%s decodes a present tagged-optional payload", (method, size, write, expected) => {
    const arena = new NativeMemoryArena();
    const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
    const ptr = arena.alloc(1 + size);
    arena.dataView.setUint8(ptr, 1);
    write(arena.dataView, ptr + 1);
    const packed = (BigInt(1 + size) << 32n) | BigInt(ptr >>> 0);
    expect((module_[method as "takePackedOptionalI32"] as (p: bigint) => unknown)(packed)).toBe(expected);
  });

  it("takePackedOptional* returns null for a tag-0 payload", () => {
    const arena = new NativeMemoryArena();
    const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
    const ptr = arena.alloc(1);
    arena.dataView.setUint8(ptr, 0);
    const packed = (BigInt(1) << 32n) | BigInt(ptr >>> 0);
    expect(module_.takePackedOptionalI32(packed)).toBeNull();
  });

  it("unpackOptionF64 treats NaN as null (NaN-boxed optional convention)", () => {
    const module_ = makeModule();
    expect(module_.unpackOptionF64(Number.NaN)).toBeNull();
    expect(module_.unpackOptionF64(3.5)).toBe(3.5);
  });
});

describe("NativeBoltFFIModule: return-slot family", () => {
  it("takeSlotU8Array reads the reserved return-slot header at offset 0", () => {
    const arena = new NativeMemoryArena();
    const module_ = new NativeBoltFFIModule({}, () => ({}), arena);
    const dataPtr = arena.alloc(4);
    arena.byteView.set([1, 2, 3, 4], dataPtr);
    arena.dataView.setUint32(0, dataPtr, true);
    arena.dataView.setUint32(4, 4, true);
    arena.dataView.setUint32(8, 4, true);
    expect(Array.from(module_.takeSlotU8Array())).toEqual([1, 2, 3, 4]);
  });

  it("takeSlotI32Array returns an empty array when the slot's ptr is 0", () => {
    const module_ = makeModule();
    expect(Array.from(module_.takeSlotI32Array())).toEqual([]);
  });
});

describe("NativeBoltFFIModule: scratch alloc + completeAsync", () => {
  it("allocScratch/freeScratch hand out arena offsets directly (struct-return-slot parity with BoltFFIModule)", () => {
    const module_ = makeModule();
    const ptr = module_.allocScratch(16);
    expect(ptr).toBeGreaterThan(0);
    expect(() => module_.freeScratch(ptr, 16)).not.toThrow();
  });

  it("completeAsync returns the callback's result when the status is written as Ok", () => {
    const module_ = makeModule();
    const result = module_.completeAsync((statusPtr) => {
      void statusPtr;
      return 99;
    });
    expect(result).toBe(99);
  });

  it("completeAsync throws BoltFFICancelledError when the callback writes status 4", () => {
    const module_ = makeModule();
    expect(() =>
      module_.completeAsync((statusPtr) => {
        // Simulate a real native completion export reporting cancellation via its FfiStatus
        // out-param, written at the arena offset `completeAsync` allocated for us.
        module_.writeToMemory(statusPtr, new Uint8Array(new Int32Array([4]).buffer));
        return undefined;
      })
    ).toThrow(/cancelled/i);
  });

  it("completeAsync throws a generic error for an unrecognized non-zero status", () => {
    const module_ = makeModule();
    expect(() =>
      module_.completeAsync((statusPtr) => {
        module_.writeToMemory(statusPtr, new Uint8Array(new Int32Array([7]).buffer));
        return undefined;
      })
    ).toThrow(/status 7/);
  });

  it("regression: completeAsync throws 'invalid argument' for status 3, matching wasm's BoltFFIModule.checkStatus (Codex finding 5, MEDIUM status semantics)", () => {
    const module_ = makeModule();
    expect(() =>
      module_.completeAsync((statusPtr) => {
        module_.writeToMemory(statusPtr, new Uint8Array(new Int32Array([3]).buffer));
        return undefined;
      })
    ).toThrow(/invalid argument/i);
  });

  it.each([
    [0, null] as const,
    [3, /invalid argument/i] as const,
    [4, /cancelled/i] as const,
    [7, /status 7/] as const,
  ])(
    "regression: checkStatus(%d) matches completeAsync's own taxonomy exactly (single source of truth, no per-route drift)",
    (status, expected) => {
      const module_ = makeModule();
      if (expected === null) {
        expect(() => module_.checkStatus(status)).not.toThrow();
      } else {
        expect(() => module_.checkStatus(status)).toThrow(expected);
      }
    }
  );
});

describe("NativeBoltFFIModule: takeLastErrorMessage", () => {
  it("returns empty string when the host exposes no last-error symbols", () => {
    const module_ = makeModule();
    expect(module_.takeLastErrorMessage()).toBe("");
  });

  it("decodes the message a mocked host writes into the arena at the given out-pointer", () => {
    const arena = new NativeMemoryArena();
    let capturedOutPtr = -1;
    const exports = {
      boltffi_last_error_message: (outPtr: number) => {
        capturedOutPtr = outPtr;
        const bytes = new TextEncoder().encode("boom");
        const strPtr = arena.alloc(bytes.length);
        arena.byteView.set(bytes, strPtr);
        arena.dataView.setUint32(outPtr, strPtr, true);
        arena.dataView.setUint32(outPtr + 4, bytes.length, true);
      },
      boltffi_free_string: () => {},
    };
    const module_ = new NativeBoltFFIModule(exports, () => ({}), arena);
    expect(module_.takeLastErrorMessage()).toBe("boom");
    expect(capturedOutPtr).toBeGreaterThanOrEqual(0);
  });
});

describe("NativeAsyncFutureManager: reentrant MaybeReady signals", () => {
  it("regression: repeated SYNCHRONOUS MaybeReady signals (a self-waking/cooperative-yield Rust future) must not recurse through the JS call stack once per re-poll (Codex finding 6, recursion)", async () => {
    // Mirrors boltffi_core's real contract exactly (continuation.rs's `store_continuation` +
    // future.rs's `RustFuture::poll`): `poll()` invokes its continuation callback SYNCHRONOUSLY,
    // at most once per call, with Ready or MaybeReady. MaybeReady fires synchronously whenever
    // `ContinuationScheduler` is already in the `Waked` state the instant `store_continuation`
    // runs -- not just a rare cross-thread race, but the deterministic, EVERY-poll outcome for any
    // future that (re)wakes itself before returning `Poll::Pending` (a legitimate pattern, e.g.
    // one built on a cooperative-yield primitive). `onSignal` re-issuing `poll()` synchronously in
    // that case, forever, recurses one JS + one "native" stack frame per iteration.
    let onSignal: ((callbackData: bigint, signal: NativeContinuationSignal) => void) | null = null;
    const manager = new NativeAsyncFutureManager<null>((signal) => {
      onSignal = signal;
      return null;
    });

    const TOTAL_ITERATIONS = 50_000;
    let pollCount = 0;
    const poll = (_handle: bigint, callbackData: bigint) => {
      pollCount += 1;
      if (pollCount >= TOTAL_ITERATIONS) {
        onSignal!(callbackData, NativeContinuationSignal.Ready);
      } else {
        onSignal!(callbackData, NativeContinuationSignal.MaybeReady);
      }
    };

    const result = await manager.pollAsyncNative(1n, poll);
    expect(result).toBe(1n);
    expect(pollCount).toBe(TOTAL_ITERATIONS);
  });
});

describe("instantiateBoltFFINative wiring", () => {
  it("pairs raw exports with a NativeAsyncFutureManager, exports staying unaugmented", () => {
    const rawExports = { boltffi_foo: () => 1 };
    const module_ = instantiateBoltFFINative(rawExports, () => ({}));
    expect(module_.exports).toBe(rawExports);
    expect(module_.asyncManager).toBeDefined();
  });

  it("regression: binds the arena's initial backing buffer to the host via __boltffi_native_bind_arena at construction time (Codex finding 3, HIGH growth binding)", () => {
    const boundBuffers: ArrayBuffer[] = [];
    const rawExports = {
      __boltffi_native_bind_arena: (buffer: ArrayBuffer) => boundBuffers.push(buffer),
    };
    const arena = new NativeMemoryArena(16);
    const module_ = new NativeBoltFFIModule(rawExports, () => ({}), arena);
    void module_;

    expect(boundBuffers).toEqual([arena.buffer]);
  });

  it("regression: re-binds the NEW backing buffer to the host every time the arena grows -- otherwise a real native adapter reads stale/freed memory for offsets issued after a grow", () => {
    const boundBuffers: ArrayBuffer[] = [];
    const rawExports = {
      __boltffi_native_bind_arena: (buffer: ArrayBuffer) => boundBuffers.push(buffer),
    };
    const arena = new NativeMemoryArena(16);
    const module_ = new NativeBoltFFIModule(rawExports, () => ({}), arena);

    module_.allocScratch(1024); // forces a grow

    expect(boundBuffers.length).toBeGreaterThanOrEqual(2);
    expect(boundBuffers.at(-1)).toBe(arena.buffer);
  });

  it("tolerates a host exports object with no __boltffi_native_bind_arena (e.g. the Bun scalar-only test harness)", () => {
    expect(() => new NativeBoltFFIModule({}, () => ({}))).not.toThrow();
  });
});
