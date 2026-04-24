import { WireReader, WireWriter } from "./wire.js";
import type { WasmWireWriterAllocator } from "./wire.js";

const FFI_BUF_DESCRIPTOR_SIZE = 16;
const FFI_STATUS_SIZE = 4;
const MIN_WRITER_CAPACITY = 64;
const MAX_WRITERS_PER_CAPACITY = 32;

export const enum WasmPollStatus {
  Pending = 0,
  Ready = 1,
  Cancelled = -1,
  Panicked = -2,
}

export class BoltFFIPanicError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "BoltFFIPanicError";
  }
}

export class BoltFFICancelledError extends Error {
  constructor() {
    super("Future was cancelled");
    this.name = "BoltFFICancelledError";
  }
}

interface PendingFuture {
  resolve: (handle: number) => void;
  reject: (error: Error) => void;
  pollSync: (handle: number) => number;
  panicMessage: (handle: number) => number;
  free: (handle: number) => void;
}

export class AsyncFutureManager {
  private pendingFutures = new Map<number, PendingFuture>();
  private wokenHandles = new Set<number>();
  private drainScheduled = false;
  private _module: BoltFFIModule | null = null;

  setModule(module: BoltFFIModule): void {
    this._module = module;
  }

  wake(handle: number): void {
    this.wokenHandles.add(handle);
    if (!this.drainScheduled) {
      this.drainScheduled = true;
      queueMicrotask(() => this.drainWakes());
    }
  }

  private drainWakes(): void {
    this.drainScheduled = false;
    const batch = [...this.wokenHandles];
    this.wokenHandles.clear();
    for (const handle of batch) {
      this.repollHandle(handle);
    }
  }

  private repollHandle(handle: number): void {
    const entry = this.pendingFutures.get(handle);
    if (!entry) return;

    const status = entry.pollSync(handle);
    if (status === WasmPollStatus.Ready) {
      this.pendingFutures.delete(handle);
      entry.resolve(handle);
    } else if (status < 0) {
      this.pendingFutures.delete(handle);
      entry.reject(this.extractAsyncError(handle, status, entry));
    }
  }

  private extractAsyncError(handle: number, status: number, entry: PendingFuture): Error {
    if (status === WasmPollStatus.Panicked && this._module) {
      const bufPtr = entry.panicMessage(handle);
      const reader = this._module.readerFromBuf(bufPtr);
      const message = reader.readString();
      this._module.freeBuf(bufPtr);
      entry.free(handle);
      return new BoltFFIPanicError(message);
    }
    entry.free(handle);
    if (status === WasmPollStatus.Cancelled) {
      return new BoltFFICancelledError();
    }
    return new Error(`Unknown poll status: ${status}`);
  }

  pollAsync(
    handle: number,
    pollSync: (handle: number) => number,
    panicMessage: (handle: number) => number,
    free: (handle: number) => void
  ): Promise<number> {
    return new Promise((resolve, reject) => {
      this.pendingFutures.set(handle, { resolve, reject, pollSync, panicMessage, free });

      const status = pollSync(handle);
      if (status === WasmPollStatus.Ready) {
        this.pendingFutures.delete(handle);
        resolve(handle);
      } else if (status < 0) {
        this.pendingFutures.delete(handle);
        reject(this.extractAsyncError(handle, status, { resolve, reject, pollSync, panicMessage, free }));
      }
    });
  }
}

export interface BoltFFIExports {
  memory: WebAssembly.Memory;
  boltffi_wasm_abi_version: () => number;
  boltffi_wasm_alloc: (size: number) => number;
  boltffi_wasm_free: (ptr: number, size: number) => void;
  boltffi_wasm_free_buf: (ptr: number, size: number, align: number) => void;
  boltffi_wasm_realloc: (ptr: number, oldSize: number, newSize: number) => number;
  boltffi_wasm_free_string_return: (ptr: number, len: number) => void;
  boltffi_wasm_return_slot_addr: () => number;
  [key: string]: WebAssembly.ExportValue;
}

export interface StringAlloc {
  ptr: number;
  len: number;
}

export interface PrimitiveBufferAlloc {
  ptr: number;
  len: number;
  allocationSize: number;
}

export type PrimitiveBufferElementType =
  | "bool"
  | "i8"
  | "u8"
  | "i16"
  | "u16"
  | "i32"
  | "u32"
  | "i64"
  | "u64"
  | "isize"
  | "usize"
  | "f32"
  | "f64";

export type WriterAlloc = WireWriter;

export class BoltFFIModule {
  readonly exports: BoltFFIExports;
  readonly asyncManager: AsyncFutureManager;
  private _memory: WebAssembly.Memory;
  private _encoder: TextEncoder;
  private _decoder: TextDecoder;
  private _writerPool: Map<number, WriterAlloc[]>;
  private _cachedU8: Uint8Array | null = null;
  private _cachedI8: Int8Array | null = null;
  private _cachedI16: Int16Array | null = null;
  private _cachedU16: Uint16Array | null = null;
  private _cachedI32: Int32Array | null = null;
  private _cachedU32: Uint32Array | null = null;
  private _cachedI64: BigInt64Array | null = null;
  private _cachedU64: BigUint64Array | null = null;
  private _cachedF32: Float32Array | null = null;
  private _cachedF64: Float64Array | null = null;
  private _cachedView: DataView | null = null;
  private _returnSlotAddr: number = 0;

  constructor(instance: WebAssembly.Instance, asyncManager: AsyncFutureManager) {
    this.exports = instance.exports as BoltFFIExports;
    this._memory = this.exports.memory;
    this._encoder = new TextEncoder();
    this._decoder = new TextDecoder("utf-8");
    this._writerPool = new Map();
    this.asyncManager = asyncManager;
    asyncManager.setModule(this);
    this._returnSlotAddr = this.exports.boltffi_wasm_return_slot_addr();
  }

  readReturnSlot(): { ptr: number; len: number; cap: number; align: number } {
    const view = this.getU32();
    const idx = this._returnSlotAddr >>> 2;
    return { ptr: view[idx], len: view[idx + 1], cap: view[idx + 2], align: view[idx + 3] };
  }

  completeAsync<T>(complete: (statusPtr: number) => T): T {
    const statusPtr = this.allocStatus();
    try {
      const result = complete(statusPtr);
      this.checkStatus(this.readStatus(statusPtr));
      return result;
    } finally {
      this.freeStatus(statusPtr);
    }
  }

  private allocStatus(): number {
    const ptr = this.exports.boltffi_wasm_alloc(FFI_STATUS_SIZE);
    if (ptr === 0) {
      throw new Error("Failed to allocate memory for status");
    }
    this.getView().setInt32(ptr, 0, true);
    return ptr;
  }

  private readStatus(ptr: number): number {
    return this.getView().getInt32(ptr, true);
  }

  private freeStatus(ptr: number): void {
    if (ptr !== 0) {
      this.exports.boltffi_wasm_free(ptr, FFI_STATUS_SIZE);
    }
  }

  private checkStatus(status: number): void {
    if (status === 0) {
      return;
    }
    if (status === 3) {
      throw new Error("invalid argument");
    }
    if (status === 4) {
      throw new BoltFFICancelledError();
    }
    throw new Error(`FFI failed in async completion with code ${status}`);
  }

  private getView(): DataView {
    if (this._cachedView === null || this._cachedView.buffer !== this._memory.buffer) {
      this._cachedView = new DataView(this._memory.buffer);
    }
    return this._cachedView;
  }

  private getBytes(): Uint8Array {
    if (this._cachedU8 === null || this._cachedU8.buffer !== this._memory.buffer) {
      this._cachedU8 = new Uint8Array(this._memory.buffer);
    }
    return this._cachedU8;
  }

  private getI8(): Int8Array {
    if (this._cachedI8 === null || this._cachedI8.buffer !== this._memory.buffer) {
      this._cachedI8 = new Int8Array(this._memory.buffer);
    }
    return this._cachedI8;
  }

  private getI16(): Int16Array {
    if (this._cachedI16 === null || this._cachedI16.buffer !== this._memory.buffer) {
      this._cachedI16 = new Int16Array(this._memory.buffer);
    }
    return this._cachedI16;
  }

  private getU16(): Uint16Array {
    if (this._cachedU16 === null || this._cachedU16.buffer !== this._memory.buffer) {
      this._cachedU16 = new Uint16Array(this._memory.buffer);
    }
    return this._cachedU16;
  }

  private getI32(): Int32Array {
    if (this._cachedI32 === null || this._cachedI32.buffer !== this._memory.buffer) {
      this._cachedI32 = new Int32Array(this._memory.buffer);
    }
    return this._cachedI32;
  }

  private getU32(): Uint32Array {
    if (this._cachedU32 === null || this._cachedU32.buffer !== this._memory.buffer) {
      this._cachedU32 = new Uint32Array(this._memory.buffer);
    }
    return this._cachedU32;
  }

  private getI64(): BigInt64Array {
    if (this._cachedI64 === null || this._cachedI64.buffer !== this._memory.buffer) {
      this._cachedI64 = new BigInt64Array(this._memory.buffer);
    }
    return this._cachedI64;
  }

  private getU64(): BigUint64Array {
    if (this._cachedU64 === null || this._cachedU64.buffer !== this._memory.buffer) {
      this._cachedU64 = new BigUint64Array(this._memory.buffer);
    }
    return this._cachedU64;
  }

  private getF32(): Float32Array {
    if (this._cachedF32 === null || this._cachedF32.buffer !== this._memory.buffer) {
      this._cachedF32 = new Float32Array(this._memory.buffer);
    }
    return this._cachedF32;
  }

  private getF64(): Float64Array {
    if (this._cachedF64 === null || this._cachedF64.buffer !== this._memory.buffer) {
      this._cachedF64 = new Float64Array(this._memory.buffer);
    }
    return this._cachedF64;
  }

  allocString(value: string): StringAlloc {
    const encoded = this._encoder.encode(value);
    const ptr = this.exports.boltffi_wasm_alloc(encoded.length);
    if (ptr === 0 && encoded.length > 0) {
      throw new Error("Failed to allocate memory for string");
    }
    this.getBytes().set(encoded, ptr);
    return { ptr, len: encoded.length };
  }

  freeAlloc(alloc: StringAlloc): void {
    if (alloc.ptr !== 0 && alloc.len !== 0) {
      this.exports.boltffi_wasm_free(alloc.ptr, alloc.len);
    }
  }

  allocBytes(value: Uint8Array): StringAlloc {
    const ptr = this.exports.boltffi_wasm_alloc(value.length);
    if (ptr === 0 && value.length > 0) {
      throw new Error("Failed to allocate memory for bytes");
    }
    this.getBytes().set(value, ptr);
    return { ptr, len: value.length };
  }

  allocI8Array(value: Int8Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Int8Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU8Array(value: Uint8Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const ptr = this.exports.boltffi_wasm_alloc(len);
    this.getBytes().set(value, ptr);
    return { ptr, len, allocationSize: len };
  }

  allocI16Array(value: Int16Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 1;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getI16().set(value, ptr >>> 1);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU16Array(value: Uint16Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 1;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getU16().set(value, ptr >>> 1);
    return { ptr, len, allocationSize: byteLen };
  }

  allocI32Array(value: Int32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 2;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getI32().set(value, ptr >>> 2);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU32Array(value: Uint32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 2;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getU32().set(value, ptr >>> 2);
    return { ptr, len, allocationSize: byteLen };
  }

  allocI64Array(value: BigInt64Array | readonly bigint[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 3;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getI64().set(value, ptr >>> 3);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU64Array(value: BigUint64Array | readonly bigint[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 3;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getU64().set(value, ptr >>> 3);
    return { ptr, len, allocationSize: byteLen };
  }

  allocF32Array(value: Float32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 2;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getF32().set(value, ptr >>> 2);
    return { ptr, len, allocationSize: byteLen };
  }

  allocF64Array(value: Float64Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len << 3;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    this.getF64().set(value, ptr >>> 3);
    return { ptr, len, allocationSize: byteLen };
  }

  allocBoolArray(value: readonly boolean[]): PrimitiveBufferAlloc {
    const len = value.length;
    const ptr = this.exports.boltffi_wasm_alloc(len);
    const view = new Uint8Array(this._memory.buffer, ptr, len);
    for (let i = 0; i < len; i++) {
      view[i] = value[i] ? 1 : 0;
    }
    return { ptr, len, allocationSize: len };
  }

  allocPrimitiveBuffer(
    value: ReadonlyArray<number | bigint | boolean>,
    elementType: PrimitiveBufferElementType
  ): PrimitiveBufferAlloc {
    const bytesPerElement = this.primitiveElementSize(elementType);
    const elementCount = value.length;
    const allocationSize = elementCount * bytesPerElement;
    const ptr = this.exports.boltffi_wasm_alloc(allocationSize);
    if (ptr === 0 && allocationSize > 0) {
      throw new Error("Failed to allocate memory for primitive buffer");
    }
    const view = this.getView();
    value.forEach((entry, index) => {
      const offset = ptr + index * bytesPerElement;
      this.writePrimitiveElement(view, offset, entry, elementType);
    });
    return { ptr, len: elementCount, allocationSize };
  }

  allocCompositeBuffer<T>(
    value: readonly T[],
    elementSize: number,
    writeElement: (writer: WriterAlloc, value: T) => void
  ): WriterAlloc {
    const writer = this.allocWriter(value.length * elementSize);
    value.forEach((entry) => writeElement(writer, entry));
    return writer;
  }

  freePrimitiveBuffer(allocation: PrimitiveBufferAlloc): void {
    if (allocation.ptr !== 0 && allocation.allocationSize !== 0) {
      this.exports.boltffi_wasm_free(allocation.ptr, allocation.allocationSize);
    }
  }

  allocWriter(size: number): WriterAlloc {
    const requestedCapacity = Math.max(size, MIN_WRITER_CAPACITY);
    const pooled = this._writerPool.get(requestedCapacity);
    if (pooled !== undefined) {
      const writer = pooled.pop();
      if (writer !== undefined) {
        writer.reset();
        return writer;
      }
    }

    const allocator: WasmWireWriterAllocator = {
      alloc: (allocationSize) => this.exports.boltffi_wasm_alloc(allocationSize),
      realloc: (ptr, oldSize, newSize) =>
        this.exports.boltffi_wasm_realloc(ptr, oldSize, newSize),
      free: (ptr, allocationSize) => this.exports.boltffi_wasm_free(ptr, allocationSize),
      buffer: () => this._memory.buffer,
    };
    return WireWriter.withWasmAllocation(requestedCapacity, allocator);
  }

  freeWriter(writer: WriterAlloc): void {
    const capacity = writer.capacity;
    writer.reset();
    const bucket = this._writerPool.get(capacity) ?? [];
    if (bucket.length < MAX_WRITERS_PER_CAPACITY) {
      bucket.push(writer);
      this._writerPool.set(capacity, bucket);
      return;
    }
    writer.release();
  }

  allocBufDescriptor(): number {
    const ptr = this.exports.boltffi_wasm_alloc(FFI_BUF_DESCRIPTOR_SIZE);
    if (ptr === 0) {
      throw new Error("Failed to allocate memory for buffer descriptor");
    }
    return ptr;
  }

  freeBufDescriptor(ptr: number): void {
    if (ptr !== 0) {
      this.exports.boltffi_wasm_free(ptr, FFI_BUF_DESCRIPTOR_SIZE);
    }
  }

  readerFromBuf(bufPtr: number): WireReader {
    const view = this.getView();
    const ptr = view.getUint32(bufPtr, true);
    return new WireReader(this._memory.buffer, ptr);
  }

  freeBuf(bufPtr: number): void {
    const { ptr, cap, align } = this.readBufDescriptor(bufPtr);
    if (ptr !== 0 && cap !== 0) {
      this.exports.boltffi_wasm_free_buf(ptr, cap, align);
    }
    this.exports.boltffi_wasm_free(bufPtr, FFI_BUF_DESCRIPTOR_SIZE);
  }

  writeBufDescriptor(bufPtr: number, dataPtr: number, dataLen: number, dataCap: number, dataAlign: number = 1): void {
    const view = this.getView();
    view.setUint32(bufPtr, dataPtr, true);
    view.setUint32(bufPtr + 4, dataLen, true);
    view.setUint32(bufPtr + 8, dataCap, true);
    view.setUint32(bufPtr + 12, dataAlign, true);
  }

  private readBufDescriptor(bufPtr: number): { ptr: number; len: number; cap: number; align: number } {
    const view = this.getView();
    return {
      ptr: view.getUint32(bufPtr, true),
      len: view.getUint32(bufPtr + 4, true),
      cap: view.getUint32(bufPtr + 8, true),
      align: view.getUint32(bufPtr + 12, true) || 1,
    };
  }

  takeBufU8Array(bufPtr: number): Uint8Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Uint8Array(0);
    return this.getBytes().subarray(ptr, ptr + len).slice();
  }

  takeBufI8Array(bufPtr: number): Int8Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Int8Array(0);
    return this.getI8().subarray(ptr, ptr + len).slice();
  }

  takeBufI16Array(bufPtr: number): Int16Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Int16Array(0);
    const elemCount = len >>> 1;
    return this.getI16().subarray(ptr >>> 1, (ptr >>> 1) + elemCount).slice();
  }

  takeBufU16Array(bufPtr: number): Uint16Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Uint16Array(0);
    const elemCount = len >>> 1;
    return this.getU16().subarray(ptr >>> 1, (ptr >>> 1) + elemCount).slice();
  }

  takeBufI32Array(bufPtr: number): Int32Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Int32Array(0);
    const elemCount = len >>> 2;
    return this.getI32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
  }

  takeBufU32Array(bufPtr: number): Uint32Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Uint32Array(0);
    const elemCount = len >>> 2;
    return this.getU32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
  }

  takeBufI64Array(bufPtr: number): BigInt64Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new BigInt64Array(0);
    const elemCount = len >>> 3;
    return this.getI64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
  }

  takeBufU64Array(bufPtr: number): BigUint64Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new BigUint64Array(0);
    const elemCount = len >>> 3;
    return this.getU64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
  }

  takeBufF32Array(bufPtr: number): Float32Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Float32Array(0);
    const elemCount = len >>> 2;
    return this.getF32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
  }

  takeBufF64Array(bufPtr: number): Float64Array {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return new Float64Array(0);
    const elemCount = len >>> 3;
    return this.getF64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
  }

  takeBufBoolArray(bufPtr: number): boolean[] {
    const { ptr, len } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return [];
    const bytes = this.getBytes().subarray(ptr, ptr + len);
    return Array.from(bytes, (value) => value !== 0);
  }

  takeBufStructArray<T>(bufPtr: number, stride: number, decode: (view: DataView, offset: number) => T): T[] {
    const { ptr, len: byteLen } = this.readBufDescriptor(bufPtr);
    if (ptr === 0) return [];
    const copy = new Uint8Array(this._memory.buffer, ptr, byteLen).slice();
    const view = new DataView(copy.buffer, copy.byteOffset, copy.byteLength);
    const count = (byteLen / stride) | 0;
    return Array.from({ length: count }, (_, index) => decode(view, index * stride));
  }

  writeToMemory(ptr: number, data: Uint8Array): void {
    this.getBytes().set(data, ptr);
  }

  readFromMemory(ptr: number, len: number): Uint8Array {
    return this.getBytes().slice(ptr, ptr + len);
  }

  private unpackPacked(packed: bigint): { pointer: number; length: number } {
    return {
      pointer: Number(packed & 0xffff_ffffn),
      length: Number((packed >> 32n) & 0xffff_ffffn),
    };
  }

  private freePacked(pointer: number, length: number): void {
    if (pointer !== 0 && length !== 0) {
      this.exports.boltffi_wasm_free_string_return(pointer, length);
    }
  }

  private takePackedOptionalPrimitive<T>(
    packed: bigint,
    encodedSize: number,
    readValue: (view: DataView, valueOffset: number) => T
  ): T | null {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) {
      return null;
    }
    const view = this.getView();
    const tag = view.getUint8(pointer);
    if (tag === 0) {
      this.freePacked(pointer, length);
      return null;
    }
    if (length < 1 + encodedSize) {
      this.freePacked(pointer, length);
      throw new Error("Invalid packed optional payload");
    }
    const value = readValue(view, pointer + 1);
    this.freePacked(pointer, length);
    return value;
  }

  takePackedOptionalBool(packed: bigint): boolean | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getUint8(offset) !== 0);
  }

  takePackedOptionalI8(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getInt8(offset));
  }

  takePackedOptionalU8(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 1, (view, offset) => view.getUint8(offset));
  }

  takePackedOptionalI16(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 2, (view, offset) => view.getInt16(offset, true));
  }

  takePackedOptionalU16(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 2, (view, offset) => view.getUint16(offset, true));
  }

  takePackedOptionalI32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getInt32(offset, true));
  }

  takePackedOptionalU32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getUint32(offset, true));
  }

  takePackedOptionalI64(packed: bigint): bigint | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getBigInt64(offset, true));
  }

  takePackedOptionalU64(packed: bigint): bigint | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getBigUint64(offset, true));
  }

  takePackedOptionalF32(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 4, (view, offset) => view.getFloat32(offset, true));
  }

  takePackedOptionalF64(packed: bigint): number | null {
    return this.takePackedOptionalPrimitive(packed, 8, (view, offset) => view.getFloat64(offset, true));
  }

  unpackOptionBool(packed: number): boolean | null {
    if (Number.isNaN(packed)) return null;
    return packed !== 0;
  }

  unpackOptionI8(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed | 0;
  }

  unpackOptionU8(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed >>> 0;
  }

  unpackOptionI16(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed | 0;
  }

  unpackOptionU16(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed >>> 0;
  }

  unpackOptionI32(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed | 0;
  }

  unpackOptionU32(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed >>> 0;
  }

  unpackOptionF32(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed;
  }

  unpackOptionF64(packed: number): number | null {
    if (Number.isNaN(packed)) return null;
    return packed;
  }

  takePackedUtf8String(packed: bigint): string {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) {
      return "";
    }
    const bytes = new Uint8Array(this._memory.buffer, pointer, length);
    try {
      return this._decoder.decode(bytes);
    } finally {
      this.freePacked(pointer, length);
    }
  }



  takePackedBuffer(packed: bigint): WireReader {
    const { pointer, length } = this.unpackPacked(packed);
    if (pointer === 0 || length === 0) {
      return new WireReader(new ArrayBuffer(0), 0);
    }
    const bytes = new Uint8Array(this._memory.buffer, pointer, length);
    const copy = bytes.slice();
    this.freePacked(pointer, length);
    return new WireReader(copy.buffer, 0);
  }

  takePackedI8Array(packed: bigint): Int8Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int8Array(0);
    const result = this.getI8().subarray(pointer, pointer + byteLen).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU8Array(packed: bigint): Uint8Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint8Array(0);
    const result = this.getBytes().subarray(pointer, pointer + byteLen).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  private readSlot(): { ptr: number; len: number; cap: number; align: number } {
    const slotView = this.getU32();
    const slotIdx = this._returnSlotAddr >>> 2;
    return {
      ptr: slotView[slotIdx],
      len: slotView[slotIdx + 1],
      cap: slotView[slotIdx + 2],
      align: slotView[slotIdx + 3] || 1,
    };
  }

  private freeSlotBuf(ptr: number, cap: number, align: number): void {
    this.exports.boltffi_wasm_free_buf(ptr, cap, align);
  }

  takeSlotU8Array(): Uint8Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Uint8Array(0);
    const result = this.getBytes().subarray(ptr, ptr + len).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotI8Array(): Int8Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Int8Array(0);
    const result = this.getI8().subarray(ptr, ptr + len).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotI32Array(): Int32Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Int32Array(0);
    const elemCount = len >>> 2;
    const result = this.getI32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotU32Array(): Uint32Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Uint32Array(0);
    const elemCount = len >>> 2;
    const result = this.getU32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotF32Array(): Float32Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Float32Array(0);
    const elemCount = len >>> 2;
    const result = this.getF32().subarray(ptr >>> 2, (ptr >>> 2) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotF64Array(): Float64Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Float64Array(0);
    const elemCount = len >>> 3;
    const result = this.getF64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotI16Array(): Int16Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Int16Array(0);
    const elemCount = len >>> 1;
    const result = this.getI16().subarray(ptr >>> 1, (ptr >>> 1) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotU16Array(): Uint16Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new Uint16Array(0);
    const elemCount = len >>> 1;
    const result = this.getU16().subarray(ptr >>> 1, (ptr >>> 1) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotI64Array(): BigInt64Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new BigInt64Array(0);
    const elemCount = len >>> 3;
    const result = this.getI64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotU64Array(): BigUint64Array {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return new BigUint64Array(0);
    const elemCount = len >>> 3;
    const result = this.getU64().subarray(ptr >>> 3, (ptr >>> 3) + elemCount).slice();
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotBoolArray(): boolean[] {
    const { ptr, len, cap, align } = this.readSlot();
    if (ptr === 0) return [];
    const bytes = this.getBytes().subarray(ptr, ptr + len);
    const result = Array.from(bytes, (b) => b !== 0);
    this.freeSlotBuf(ptr, cap, align);
    return result;
  }

  takeSlotStructArray<T>(stride: number, decode: (view: DataView, offset: number) => T): T[] {
    const { ptr, len: byteLen, cap, align } = this.readSlot();
    if (ptr === 0) return [];
    const count = (byteLen / stride) | 0;
    const copy = new Uint8Array(this._memory.buffer, ptr, byteLen).slice();
    this.freeSlotBuf(ptr, cap, align);
    const view = new DataView(copy.buffer, copy.byteOffset, copy.byteLength);
    const result: T[] = new Array(count);
    for (let i = 0; i < count; i++) {
      result[i] = decode(view, i * stride);
    }
    return result;
  }

  takePackedI16Array(packed: bigint): Int16Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int16Array(0);
    const elemCount = byteLen / 2;
    const result = new Int16Array(this._memory.buffer, pointer, elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU16Array(packed: bigint): Uint16Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint16Array(0);
    const elemCount = byteLen / 2;
    const result = new Uint16Array(this._memory.buffer, pointer, elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedI32Array(packed: bigint): Int32Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int32Array(0);
    const elemCount = byteLen / 4;
    const result = this.getI32().subarray(pointer / 4, pointer / 4 + elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU32Array(packed: bigint): Uint32Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint32Array(0);
    const elemCount = byteLen / 4;
    const result = this.getU32().subarray(pointer / 4, pointer / 4 + elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedI64Array(packed: bigint): BigInt64Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new BigInt64Array(0);
    const result = new BigInt64Array(this._memory.buffer, pointer, byteLen / 8).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU64Array(packed: bigint): BigUint64Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new BigUint64Array(0);
    const result = new BigUint64Array(this._memory.buffer, pointer, byteLen / 8).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedF32Array(packed: bigint): Float32Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Float32Array(0);
    const elemCount = byteLen / 4;
    const result = this.getF32().subarray(pointer / 4, pointer / 4 + elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedF64Array(packed: bigint): Float64Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Float64Array(0);
    const elemCount = byteLen / 8;
    const result = this.getF64().subarray(pointer / 8, pointer / 8 + elemCount).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  private primitiveElementSize(elementType: PrimitiveBufferElementType): number {
    switch (elementType) {
      case "bool":
      case "i8":
      case "u8":
        return 1;
      case "i16":
      case "u16":
        return 2;
      case "i32":
      case "u32":
      case "isize":
      case "usize":
      case "f32":
        return 4;
      case "i64":
      case "u64":
      case "f64":
        return 8;
    }
  }

  private writePrimitiveElement(
    view: DataView,
    offset: number,
    value: number | bigint | boolean,
    elementType: PrimitiveBufferElementType
  ): void {
    switch (elementType) {
      case "bool":
        view.setUint8(offset, value ? 1 : 0);
        return;
      case "i8":
        view.setInt8(offset, Number(value));
        return;
      case "u8":
        view.setUint8(offset, Number(value));
        return;
      case "i16":
        view.setInt16(offset, Number(value), true);
        return;
      case "u16":
        view.setUint16(offset, Number(value), true);
        return;
      case "i32":
      case "isize":
        view.setInt32(offset, Number(value), true);
        return;
      case "u32":
      case "usize":
        view.setUint32(offset, Number(value), true);
        return;
      case "i64":
        view.setBigInt64(offset, BigInt(value), true);
        return;
      case "u64":
        view.setBigUint64(offset, BigInt(value), true);
        return;
      case "f32":
        view.setFloat32(offset, Number(value), true);
        return;
      case "f64":
        view.setFloat64(offset, Number(value), true);
        return;
    }
  }
}

export interface BoltFFIImports {
  env?: Record<string, WebAssembly.ImportValue>;
}

function createUnimplementedImport(importName: string): (...args: unknown[]) => never {
  return () => {
    throw new Error(`Unimplemented wasm import: ${importName}`);
  };
}

function createImportModuleProxy(moduleName: string): Record<string, WebAssembly.ImportValue> {
  return new Proxy(
    {},
    {
      get: (_target, propertyName) =>
        createUnimplementedImport(`${moduleName}.${String(propertyName)}`),
    }
  );
}

export async function instantiateBoltFFI(
  source: BufferSource | Response,
  expectedVersion: number,
  imports?: BoltFFIImports
): Promise<BoltFFIModule> {
  let wasmSource: BufferSource;

  if (source instanceof Response) {
    wasmSource = await source.arrayBuffer();
  } else {
    wasmSource = source;
  }

  const asyncManager = new AsyncFutureManager();

  const importObject: WebAssembly.Imports = {
    env: {
      __boltffi_wake: (handle: number) => asyncManager.wake(handle),
      ...(imports?.env ?? {}),
    },
    __wbindgen_placeholder__: createImportModuleProxy("__wbindgen_placeholder__"),
    __wbindgen_externref_xform__: createImportModuleProxy("__wbindgen_externref_xform__"),
  };

  const { instance } = await WebAssembly.instantiate(wasmSource, importObject);
  const module = new BoltFFIModule(instance, asyncManager);

  const actualVersion = module.exports.boltffi_wasm_abi_version();
  if (actualVersion !== expectedVersion) {
    throw new Error(
      `BoltFFI ABI version mismatch: expected ${expectedVersion}, got ${actualVersion}`
    );
  }

  return module;
}

export function instantiateBoltFFISync(
  source: BufferSource,
  expectedVersion: number,
  imports?: BoltFFIImports
): BoltFFIModule {
  const asyncManager = new AsyncFutureManager();

  const importObject: WebAssembly.Imports = {
    env: {
      __boltffi_wake: (handle: number) => asyncManager.wake(handle),
      ...(imports?.env ?? {}),
    },
    __wbindgen_placeholder__: createImportModuleProxy("__wbindgen_placeholder__"),
    __wbindgen_externref_xform__: createImportModuleProxy("__wbindgen_externref_xform__"),
  };

  const wasmModule = new WebAssembly.Module(source);
  const instance = new WebAssembly.Instance(wasmModule, importObject);
  const module = new BoltFFIModule(instance, asyncManager);

  const actualVersion = module.exports.boltffi_wasm_abi_version();
  if (actualVersion !== expectedVersion) {
    throw new Error(
      `BoltFFI ABI version mismatch: expected ${expectedVersion}, got ${actualVersion}`
    );
  }

  return module;
}
