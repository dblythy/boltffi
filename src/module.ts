import { WireReader, WireWriter } from "./wire.js";
import type { WasmWireWriterAllocator } from "./wire.js";

const FFI_BUF_DESCRIPTOR_SIZE = 12;

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
  boltffi_wasm_realloc: (ptr: number, oldSize: number, newSize: number) => number;
  boltffi_wasm_free_string_return: (ptr: number, len: number) => void;
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

  constructor(instance: WebAssembly.Instance, asyncManager: AsyncFutureManager) {
    this.exports = instance.exports as BoltFFIExports;
    this._memory = this.exports.memory;
    this._encoder = new TextEncoder();
    this._decoder = new TextDecoder("utf-8");
    this.asyncManager = asyncManager;
    asyncManager.setModule(this);
  }

  private getView(): DataView {
    return new DataView(this._memory.buffer);
  }

  private getBytes(): Uint8Array {
    return new Uint8Array(this._memory.buffer);
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
    const byteLen = len;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Uint8Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocI16Array(value: Int16Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 2;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Int16Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU16Array(value: Uint16Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 2;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Uint16Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocI32Array(value: Int32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 4;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Int32Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU32Array(value: Uint32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 4;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Uint32Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocI64Array(value: BigInt64Array | readonly bigint[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 8;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new BigInt64Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocU64Array(value: BigUint64Array | readonly bigint[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 8;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new BigUint64Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocF32Array(value: Float32Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 4;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Float32Array(this._memory.buffer, ptr, len).set(value);
    return { ptr, len, allocationSize: byteLen };
  }

  allocF64Array(value: Float64Array | readonly number[]): PrimitiveBufferAlloc {
    const len = value.length;
    const byteLen = len * 8;
    const ptr = this.exports.boltffi_wasm_alloc(byteLen);
    new Float64Array(this._memory.buffer, ptr, len).set(value);
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

  freePrimitiveBuffer(allocation: PrimitiveBufferAlloc): void {
    if (allocation.ptr !== 0 && allocation.allocationSize !== 0) {
      this.exports.boltffi_wasm_free(allocation.ptr, allocation.allocationSize);
    }
  }

  allocWriter(size: number): WriterAlloc {
    const allocator: WasmWireWriterAllocator = {
      alloc: (allocationSize) => this.exports.boltffi_wasm_alloc(allocationSize),
      realloc: (ptr, oldSize, newSize) =>
        this.exports.boltffi_wasm_realloc(ptr, oldSize, newSize),
      free: (ptr, allocationSize) => this.exports.boltffi_wasm_free(ptr, allocationSize),
      buffer: () => this._memory.buffer,
    };
    return WireWriter.withWasmAllocation(Math.max(size, 64), allocator);
  }

  freeWriter(writer: WriterAlloc): void {
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
    const view = this.getView();
    const ptr = view.getUint32(bufPtr, true);
    const cap = view.getUint32(bufPtr + 8, true);
    if (ptr !== 0 && cap !== 0) {
      this.exports.boltffi_wasm_free(ptr, cap);
    }
    this.exports.boltffi_wasm_free(bufPtr, FFI_BUF_DESCRIPTOR_SIZE);
  }

  writeBufDescriptor(bufPtr: number, dataPtr: number, dataLen: number, dataCap: number): void {
    const view = this.getView();
    view.setUint32(bufPtr, dataPtr, true);
    view.setUint32(bufPtr + 4, dataLen, true);
    view.setUint32(bufPtr + 8, dataCap, true);
  }

  writeToMemory(ptr: number, data: Uint8Array): void {
    this.getBytes().set(data, ptr);
  }

  readFromMemory(ptr: number, len: number): Uint8Array {
    return this.getBytes().slice(ptr, ptr + len);
  }

  takePackedUtf8String(packed: bigint): string {
    const pointer = Number(packed & 0xffff_ffffn);
    const length = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || length === 0) {
      return "";
    }
    const bytes = new Uint8Array(this._memory.buffer, pointer, length);
    try {
      return this._decoder.decode(bytes);
    } finally {
      this.exports.boltffi_wasm_free_string_return(pointer, length);
    }
  }



  takePackedBuffer(packed: bigint): WireReader {
    const pointer = Number(packed & 0xffff_ffffn);
    const length = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || length === 0) {
      return new WireReader(new ArrayBuffer(0), 0);
    }
    const bytes = new Uint8Array(this._memory.buffer, pointer, length);
    const copy = bytes.slice();
    this.exports.boltffi_wasm_free_string_return(pointer, length);
    return new WireReader(copy.buffer, 0);
  }

  takePackedI8Array(packed: bigint): Int8Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int8Array(0);
    const result = new Int8Array(this._memory.buffer, pointer, byteLen).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU8Array(packed: bigint): Uint8Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint8Array(0);
    const result = new Uint8Array(this._memory.buffer, pointer, byteLen).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedI16Array(packed: bigint): Int16Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int16Array(0);
    const result = new Int16Array(this._memory.buffer, pointer, byteLen / 2).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU16Array(packed: bigint): Uint16Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint16Array(0);
    const result = new Uint16Array(this._memory.buffer, pointer, byteLen / 2).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedI32Array(packed: bigint): Int32Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Int32Array(0);
    const result = new Int32Array(this._memory.buffer, pointer, byteLen / 4).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedU32Array(packed: bigint): Uint32Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Uint32Array(0);
    const result = new Uint32Array(this._memory.buffer, pointer, byteLen / 4).slice();
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
    const result = new Float32Array(this._memory.buffer, pointer, byteLen / 4).slice();
    this.exports.boltffi_wasm_free_string_return(pointer, byteLen);
    return result;
  }

  takePackedF64Array(packed: bigint): Float64Array {
    const pointer = Number(packed & 0xffff_ffffn);
    const byteLen = Number((packed >> 32n) & 0xffff_ffffn);
    if (pointer === 0 || byteLen === 0) return new Float64Array(0);
    const result = new Float64Array(this._memory.buffer, pointer, byteLen / 8).slice();
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
