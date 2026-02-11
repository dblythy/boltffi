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

  writeToMemory(ptr: number, data: Uint8Array): void {
    this.getBytes().set(data, ptr);
  }

  readFromMemory(ptr: number, len: number): Uint8Array {
    return this.getBytes().slice(ptr, ptr + len);
  }
}

export async function instantiateBoltFFI(
  source: BufferSource | Response,
  expectedVersion: number
): Promise<BoltFFIModule> {
  let wasmSource: BufferSource;

  if (source instanceof Response) {
    wasmSource = await source.arrayBuffer();
  } else {
    wasmSource = source;
  }

  const { instance } = await WebAssembly.instantiate(wasmSource);
  const module = new BoltFFIModule(instance);

  const actualVersion = module.exports.boltffi_wasm_abi_version();
  if (actualVersion !== expectedVersion) {
    throw new Error(
      `BoltFFI ABI version mismatch: expected ${expectedVersion}, got ${actualVersion}`
    );
  }

  return module;
}
