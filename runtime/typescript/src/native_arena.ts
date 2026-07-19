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
 */
export class NativeMemoryArena {
  private buf: ArrayBuffer;
  private view: DataView;
  private bytes: Uint8Array;
  private readonly freeBlocks: FreeBlock[] = [];
  private highWaterMark: number;
  private onGrow: ((buffer: ArrayBuffer) => void) | null = null;

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
    const fitIndex = this.freeBlocks.findIndex((block) => block.len >= size);
    if (fitIndex !== -1) {
      const block = this.freeBlocks[fitIndex];
      const ptr = block.ptr;
      if (block.len === size) {
        this.freeBlocks.splice(fitIndex, 1);
      } else {
        block.ptr += size;
        block.len -= size;
      }
      return ptr;
    }

    if (this.highWaterMark + size > this.buf.byteLength) {
      this.grow(this.highWaterMark + size);
    }
    const ptr = this.highWaterMark;
    this.highWaterMark += size;
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
