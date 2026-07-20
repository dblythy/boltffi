// The react-native track's seam items 2/2b/3 (docs/tracks/react-native.md, parse-core-sdks repo):
// a pure-JS, backend-owned "linear memory" simulation for the native backend, mirroring what
// `WebAssembly.Memory` gives the wasm backend for free. There is no wasm module here and no real
// native allocator call needed for this session's scope -- everything below is plain
// `ArrayBuffer`/`DataView` arithmetic, exactly like `WireWriter`'s existing non-wasm ("local")
// mode already does for its own growth (`wire.ts`'s `ensureCapacity`).
//
// Why an arena exists at all, given native process memory has "no growth-invalidation to work
// around" (the design doc's own framing): the *codegen* that calls into this runtime is
// backend-agnostic -- the same `TsParam::wrapper_code()`/`cleanup_code()` Rust logic that emits
// `_module.allocString(...)`/`_module.allocWriter(...)`/`_module.freeWriter(...)` runs for both
// wasm and native targets, and a handful of templates (`class.txt`, `callback.txt`,
// `value_type_companion.txt`, `enum_namespace.txt`, `function.txt`) call `_module.writeBufDescriptor`/
// `WireWriter.ptr` expecting a plain JS `number` "pointer" either way. Reusing that exact
// numeric-offset contract here -- instead of inventing a parallel "pass the raw buffer value"
// calling convention just for native -- means ALL of those call sites keep working against a
// `NativeBoltFFIModule` with zero codegen changes (see native.ts's module doc). The arena is the
// backing store that numeric contract needs.
//
// In a real JSI adapter, this arena's backing `ArrayBuffer` would be created once (and re-shared
// on every grow) with the C++ HostObject via a reserved `exports.__boltffi_native_bind_arena`
// call (see `NativeBoltFFIModule`'s constructor) so a `jsi::ArrayBuffer`'s real backing pointer
// plus a JS-tracked offset gives C++ everything it needs to translate an arena offset into a real
// native address for the one-shot duration of an FFI call. Nothing in this file assumes that
// binding exists -- tests exercise it exactesly as a plain, host-less byte arena.

/** A single free block, kept sorted by `ptr` so adjacent-free coalescing is a neighbor check. */
interface FreeBlock {
  ptr: number;
  len: number;
}

const DEFAULT_INITIAL_SIZE = 4096;
/** Every allocation is rounded up to this many bytes, both in address and reserved size -- the
 * native side reads decoded values (i64/f64 arrays, the 16-byte buf-descriptor/return-slot
 * layout) as typed pointers through a real native ABI, where an unaligned read is UB (unlike a
 * JS `DataView`, which tolerates any byte offset). Rounding the SIZE up too (not just the
 * returned pointer) keeps every free-list block's `ptr` a multiple of `ALIGNMENT` forever, even
 * after a partial split hands back the front of a larger block -- `ptr + alignedSize` stays
 * aligned because both operands are. 8 covers every primitive this runtime ever hands the native
 * side (i64/u64/f64 are the widest). */
const ALIGNMENT = 8;

function alignUp(n: number): number {
  return (n + ALIGNMENT - 1) & ~(ALIGNMENT - 1);
}

/** Reserved header region at offset 0, mirroring wasm's `boltffi_wasm_return_slot_addr` -- a
 * fixed scratch slot for the one active "return slot" struct decode in flight. Never handed out
 * by `alloc`. */
export const RETURN_SLOT_SIZE = 16;

/**
 * A growable, first-fit free-list byte allocator over a plain `ArrayBuffer` -- the native
 * backend's counterpart to a wasm module's linear memory. `alloc`/`free`/`realloc` mirror
 * `boltffi_wasm_alloc`/`_free`/`_realloc`'s signatures exactly (same units: byte offsets, byte
 * lengths) so `NativeBoltFFIModule` can reuse `BoltFFIModule`'s own algorithms verbatim, just
 * swapping which allocator backs them.
 *
 * Reentrant growth (second adversarial round's finding 2, HIGH): the runtime/cpp JSI adapter
 * (`boltffi_generic_host_object.cpp`) computes a real address for every arena-offset pointer
 * argument BEFORE invoking the native function, then holds that address for the call's entire
 * duration. If the native call is synchronous and reenters JS before returning -- a real,
 * concrete path: `subscribe_with_listener(..., int32_t* return_out)` synchronously invokes
 * `LiveQueryTransport::send` (parse-core-rs's `livequery.rs`), a host callback the JS side
 * implements -- and that reentrant JS code allocates enough to force `grow()`, the backing
 * buffer is REPLACED mid-call. The C++ side pins the OLD buffer object alive (so the write itself
 * is memory-safe, never a use-after-free), but Rust still writes the request id through the
 * ALREADY-TRANSLATED address into that OLD buffer -- coherence, not lifetime, is what breaks:
 * once the outer call returns, JS reads the same offset back out of `this.buf`, which by then is
 * the NEW buffer, and finds whatever was copied there at grow time (stale/zero), never the value
 * Rust just wrote.
 *
 * The fix is `beginCall()`/`endCall()`: for as long as ANY call is marked in flight, `grow()`
 * refuses to run at all, throwing instead of silently reallocating. This was chosen over (a)
 * post-call write-back reconciliation (copying out-param regions old-to-new after the call --
 * needs precise, fragile region tracking this arena has no way to know) or (b) routing every
 * out-param through a separate non-growable side-buffer (indirection every call site would need
 * to thread through) because it needs no new bookkeeping, is enforceable entirely on the JS side
 * with the arena's own existing state, and turns a silent data-corruption bug into a loud,
 * immediate failure at the exact moment coherence would otherwise be lost -- the honest answer
 * when reentrant growth genuinely can't be made coherent for free is to refuse it, not paper over
 * it. Wiring `beginCall()`/`endCall()` around the actual native-call dispatch (native.ts's
 * `NativeBoltFFIModule`, and the equivalent point in the C++ HostObject) is a follow-up outside
 * this file's scope; this class only owns the enforced primitive and its own tests.
 */
export class NativeMemoryArena {
  private buf: ArrayBuffer;
  private view: DataView;
  private bytes: Uint8Array;
  private readonly freeBlocks: FreeBlock[] = [];
  private highWaterMark: number;
  private onGrow: ((buffer: ArrayBuffer) => void) | null = null;
  private callDepth = 0;

  constructor(initialSize: number = DEFAULT_INITIAL_SIZE) {
    const size = Math.max(initialSize, RETURN_SLOT_SIZE);
    this.buf = new ArrayBuffer(size);
    this.view = new DataView(this.buf);
    this.bytes = new Uint8Array(this.buf);
    this.highWaterMark = RETURN_SLOT_SIZE;
  }

  /** Registers a callback fired every time the backing buffer is replaced by a larger one (a
   * real adapter re-shares the new buffer with its native side through this hook; tests may
   * ignore it entirely). */
  setOnGrow(callback: (buffer: ArrayBuffer) => void): void {
    this.onGrow = callback;
  }

  /**
   * Marks the start of a native call whose arena-offset arguments the caller is ABOUT TO
   * translate into real addresses against the CURRENT backing buffer (or already has). Nested --
   * a reentrant call invoked from a synchronous host-callback the outer call triggers (e.g. Rust's
   * `LiveQueryTransport::send`) calls this again, and growth stays forbidden until the OUTERMOST
   * `endCall()` runs. Must always be paired with `endCall()`, in a `try`/`finally` around the
   * actual native invocation -- see the class doc's "reentrant growth" note for why.
   */
  beginCall(): void {
    this.callDepth++;
  }

  /**
   * Ends one call marked by `beginCall()`. Throws if called without a matching `beginCall()` --
   * mismatched bookkeeping is a caller bug (a missing `try`/`finally`, or a double `endCall()`)
   * that must fail loudly rather than silently under/over-count and leave the growth guard either
   * stuck forever or disabled while a call is still active.
   */
  endCall(): void {
    if (this.callDepth === 0) {
      throw new Error("boltffi: NativeMemoryArena.endCall() called without a matching beginCall()");
    }
    this.callDepth--;
  }

  get buffer(): ArrayBuffer {
    return this.buf;
  }

  get dataView(): DataView {
    return this.view;
  }

  get byteView(): Uint8Array {
    return this.bytes;
  }

  get returnSlotAddr(): number {
    return 0;
  }

  alloc(size: number): number {
    if (size === 0) return 0;
    const alignedSize = alignUp(size);

    const fitIndex = this.freeBlocks.findIndex((block) => block.len >= alignedSize);
    if (fitIndex !== -1) {
      const block = this.freeBlocks[fitIndex];
      const ptr = block.ptr;
      if (block.len === alignedSize) {
        this.freeBlocks.splice(fitIndex, 1);
      } else {
        // `ptr` was already a multiple of ALIGNMENT (every block ptr ever handed out is), and
        // `alignedSize` is one too, so the residual block's ptr stays aligned.
        block.ptr += alignedSize;
        block.len -= alignedSize;
      }
      return ptr;
    }

    const ptr = alignUp(this.highWaterMark);
    if (ptr + alignedSize > this.buf.byteLength) {
      this.grow(ptr + alignedSize);
    }
    this.highWaterMark = ptr + alignedSize;
    return ptr;
  }

  free(ptr: number, size: number): void {
    if (ptr === 0 || size === 0) return;
    this.freeBlocks.push({ ptr, len: size });
    this.freeBlocks.sort((a, b) => a.ptr - b.ptr);
    this.coalesce();
  }

  realloc(ptr: number, oldSize: number, newSize: number): number {
    if (newSize <= oldSize) {
      return ptr;
    }
    const newPtr = this.alloc(newSize);
    this.bytes.copyWithin(newPtr, ptr, ptr + oldSize);
    // Re-read after copyWithin in case alloc() grew (and thus replaced) the buffer mid-call --
    // copyWithin above already ran against the up-to-date `this.bytes` since alloc() happens
    // first and grow() reassigns `this.bytes` synchronously.
    this.free(ptr, oldSize);
    return newPtr;
  }

  private grow(minSize: number): void {
    if (this.callDepth > 0) {
      // Forbid growth during an active native call rather than silently reallocating (design
      // chosen over post-call write-back reconciliation or a separate stable side-buffer for
      // out-params: this is the option enforceable entirely on the JS side, with no new region-
      // tracking machinery, and it fails LOUDLY at the exact moment coherence would otherwise be
      // silently lost) -- see the class doc's "reentrant growth" note for the concrete scenario
      // (`subscribe_with_listener`'s synchronous `transport.send` reentering JS) this closes.
      throw new Error(
        "boltffi: cannot grow the native arena while a native call is in flight -- a pointer " +
          "argument already translated against the CURRENT buffer (or an out-param the callee " +
          "still needs to write through) would silently end up pointing into the buffer being " +
          "replaced, and JS would read the offset back out of the NEW one afterward. Reserve " +
          "enough headroom before the call begins instead of growing reentrantly."
      );
    }
    let newSize = this.buf.byteLength;
    while (newSize < minSize) {
      newSize *= 2;
    }
    const newBuf = new ArrayBuffer(newSize);
    new Uint8Array(newBuf).set(this.bytes);
    this.buf = newBuf;
    this.view = new DataView(newBuf);
    this.bytes = new Uint8Array(newBuf);
    this.onGrow?.(newBuf);
  }

  private coalesce(): void {
    for (let i = 0; i < this.freeBlocks.length - 1; ) {
      const current = this.freeBlocks[i];
      const next = this.freeBlocks[i + 1];
      if (current.ptr + current.len === next.ptr) {
        current.len += next.len;
        this.freeBlocks.splice(i + 1, 1);
      } else {
        i += 1;
      }
    }
  }
}
