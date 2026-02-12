import { WireReader, WireWriter } from "./wire.js";
import type { WasmWireWriterAllocator } from "./wire.js";

const FFI_BUF_DESCRIPTOR_SIZE = 12;

export interface BoltFFIExports {
  memory: WebAssembly.Memory;
  boltffi_wasm_abi_version: () => number;
  boltffi_wasm_alloc: (size: number) => number;
  boltffi_wasm_free: (ptr: number, size: number) => void;
  boltffi_wasm_realloc: (ptr: number, oldSize: number, newSize: number) => number;
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
  private _memory: WebAssembly.Memory;
  private _encoder: TextEncoder;

  constructor(instance: WebAssembly.Instance) {
    this.exports = instance.exports as BoltFFIExports;
    this._memory = this.exports.memory;
    this._encoder = new TextEncoder();
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
    const len = view.getUint32(bufPtr + 4, true);
    const bytes = this.getBytes().slice(ptr, ptr + len);
    return new WireReader(bytes.buffer);
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

  const importObject: WebAssembly.Imports = {};
  if (imports?.env) {
    importObject.env = imports.env;
  }
  const { instance } = await WebAssembly.instantiate(wasmSource, importObject);
  const module = new BoltFFIModule(instance);

  const actualVersion = module.exports.boltffi_wasm_abi_version();
  if (actualVersion !== expectedVersion) {
    throw new Error(
      `BoltFFI ABI version mismatch: expected ${expectedVersion}, got ${actualVersion}`
    );
  }

  return module;
}
