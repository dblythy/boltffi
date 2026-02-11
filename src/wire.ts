export class WireReader {
  private view: DataView;
  private offset: number;

  constructor(buffer: ArrayBuffer, offset = 0) {
    this.view = new DataView(buffer);
    this.offset = offset;
  }

  readBool(): boolean {
    const value = this.view.getUint8(this.offset);
    this.offset += 1;
    return value !== 0;
  }

  readI8(): number {
    const value = this.view.getInt8(this.offset);
    this.offset += 1;
    return value;
  }

  readU8(): number {
    const value = this.view.getUint8(this.offset);
    this.offset += 1;
    return value;
  }

  readI16(): number {
    const value = this.view.getInt16(this.offset, true);
    this.offset += 2;
    return value;
  }

  readU16(): number {
    const value = this.view.getUint16(this.offset, true);
    this.offset += 2;
    return value;
  }

  readI32(): number {
    const value = this.view.getInt32(this.offset, true);
    this.offset += 4;
    return value;
  }

  readU32(): number {
    const value = this.view.getUint32(this.offset, true);
    this.offset += 4;
    return value;
  }

  readI64(): bigint {
    const value = this.view.getBigInt64(this.offset, true);
    this.offset += 8;
    return value;
  }

  readU64(): bigint {
    const value = this.view.getBigUint64(this.offset, true);
    this.offset += 8;
    return value;
  }

  readISize(): bigint {
    return this.readI64();
  }

  readUSize(): bigint {
    return this.readU64();
  }

  readF32(): number {
    const value = this.view.getFloat32(this.offset, true);
    this.offset += 4;
    return value;
  }

  readF64(): number {
    const value = this.view.getFloat64(this.offset, true);
    this.offset += 8;
    return value;
  }

  readString(): string {
    const len = this.readU32();
    const bytes = new Uint8Array(this.view.buffer, this.offset, len);
    this.offset += len;
    return new TextDecoder().decode(bytes);
  }

  readBytes(): Uint8Array {
    const len = this.readU32();
    const bytes = new Uint8Array(this.view.buffer, this.offset, len);
    this.offset += len;
    return bytes.slice();
  }

  readOptional<T>(readValue: () => T): T | null {
    const tag = this.readU8();
    if (tag === 0) {
      return null;
    }
    return readValue();
  }

  readArray<T>(readElement: () => T): T[] {
    const len = this.readU32();
    const result: T[] = [];
    for (let i = 0; i < len; i++) {
      result.push(readElement());
    }
    return result;
  }

  readResult<T, E>(readOk: () => T, readErr: () => E): T {
    const tag = this.readU8();
    if (tag === 0) {
      return readOk();
    }
    throw readErr();
  }
}

export type WireOk<T> = { tag: "ok"; value: T };
export type WireErr<E> = { tag: "err"; error: E };
export type WireResult<T, E> = WireOk<T> | WireErr<E>;

export interface WasmWireWriterAllocator {
  alloc(size: number): number;
  realloc(ptr: number, oldSize: number, newSize: number): number;
  free(ptr: number, size: number): void;
  buffer(): ArrayBuffer;
}

export function wireOk<T>(value: T): WireOk<T> {
  return { tag: "ok", value };
}

export function wireErr<E>(error: E): WireErr<E> {
  return { tag: "err", error };
}

export class WireWriter {
  private localBuffer: ArrayBuffer;
  private localView: DataView;
  private wasmAllocator: WasmWireWriterAllocator | null;
  private wasmPtr: number;
  private allocationSize: number;
  private offset: number;

  constructor(initialSize = 256) {
    const normalizedSize = Math.max(initialSize, 1);
    this.localBuffer = new ArrayBuffer(normalizedSize);
    this.localView = new DataView(this.localBuffer);
    this.wasmAllocator = null;
    this.wasmPtr = 0;
    this.allocationSize = normalizedSize;
    this.offset = 0;
  }

  static withWasmAllocation(
    initialSize: number,
    allocator: WasmWireWriterAllocator
  ): WireWriter {
    const normalizedSize = Math.max(initialSize, 1);
    const pointer = allocator.alloc(normalizedSize);
    if (pointer === 0 && normalizedSize > 0) {
      throw new Error("Failed to allocate memory for writer");
    }
    const writer = new WireWriter(1);
    writer.wasmAllocator = allocator;
    writer.wasmPtr = pointer;
    writer.allocationSize = normalizedSize;
    return writer;
  }

  release(): void {
    if (this.wasmAllocator !== null && this.wasmPtr !== 0 && this.allocationSize !== 0) {
      this.wasmAllocator.free(this.wasmPtr, this.allocationSize);
      this.wasmPtr = 0;
      this.allocationSize = 0;
      this.offset = 0;
    }
  }

  get capacity(): number {
    return this.allocationSize;
  }

  private inWasmMemory(): boolean {
    return this.wasmAllocator !== null;
  }

  private currentBuffer(): ArrayBuffer {
    return this.inWasmMemory() ? this.wasmAllocator!.buffer() : this.localBuffer;
  }

  private currentView(): DataView {
    return this.inWasmMemory() ? new DataView(this.wasmAllocator!.buffer()) : this.localView;
  }

  private writePosition(): number {
    return this.inWasmMemory() ? this.wasmPtr + this.offset : this.offset;
  }

  private ensureCapacity(additionalBytes: number): void {
    if (this.allocationSize === 0) {
      throw new Error("Cannot write using a released WireWriter");
    }
    const required = this.offset + additionalBytes;
    if (required <= this.allocationSize) {
      return;
    }
    let newSize = this.allocationSize;
    while (newSize < required) {
      newSize *= 2;
    }
    if (this.inWasmMemory()) {
      const newPointer = this.wasmAllocator!.realloc(this.wasmPtr, this.allocationSize, newSize);
      if (newPointer === 0 && newSize > 0) {
        throw new Error("Failed to reallocate memory for writer");
      }
      this.wasmPtr = newPointer;
      this.allocationSize = newSize;
      return;
    }
    const newBuffer = new ArrayBuffer(newSize);
    new Uint8Array(newBuffer).set(new Uint8Array(this.localBuffer));
    this.localBuffer = newBuffer;
    this.localView = new DataView(this.localBuffer);
    this.allocationSize = newSize;
  }

  get ptr(): number {
    return this.wasmPtr;
  }

  get len(): number {
    return this.offset;
  }

  getBytes(): Uint8Array {
    const start = this.inWasmMemory() ? this.wasmPtr : 0;
    return new Uint8Array(this.currentBuffer(), start, this.offset).slice();
  }

  writeBool(value: boolean): void {
    this.ensureCapacity(1);
    this.currentView().setUint8(this.writePosition(), value ? 1 : 0);
    this.offset += 1;
  }

  writeI8(value: number): void {
    this.ensureCapacity(1);
    this.currentView().setInt8(this.writePosition(), value);
    this.offset += 1;
  }

  writeU8(value: number): void {
    this.ensureCapacity(1);
    this.currentView().setUint8(this.writePosition(), value);
    this.offset += 1;
  }

  writeI16(value: number): void {
    this.ensureCapacity(2);
    this.currentView().setInt16(this.writePosition(), value, true);
    this.offset += 2;
  }

  writeU16(value: number): void {
    this.ensureCapacity(2);
    this.currentView().setUint16(this.writePosition(), value, true);
    this.offset += 2;
  }

  writeI32(value: number): void {
    this.ensureCapacity(4);
    this.currentView().setInt32(this.writePosition(), value, true);
    this.offset += 4;
  }

  writeU32(value: number): void {
    this.ensureCapacity(4);
    this.currentView().setUint32(this.writePosition(), value, true);
    this.offset += 4;
  }

  writeI64(value: bigint): void {
    this.ensureCapacity(8);
    this.currentView().setBigInt64(this.writePosition(), value, true);
    this.offset += 8;
  }

  writeU64(value: bigint): void {
    this.ensureCapacity(8);
    this.currentView().setBigUint64(this.writePosition(), value, true);
    this.offset += 8;
  }

  writeISize(value: bigint): void {
    this.writeI64(value);
  }

  writeUSize(value: bigint): void {
    this.writeU64(value);
  }

  writeF32(value: number): void {
    this.ensureCapacity(4);
    this.currentView().setFloat32(this.writePosition(), value, true);
    this.offset += 4;
  }

  writeF64(value: number): void {
    this.ensureCapacity(8);
    this.currentView().setFloat64(this.writePosition(), value, true);
    this.offset += 8;
  }

  writeString(value: string): void {
    const encoded = new TextEncoder().encode(value);
    this.writeU32(encoded.length);
    this.ensureCapacity(encoded.length);
    new Uint8Array(this.currentBuffer()).set(encoded, this.writePosition());
    this.offset += encoded.length;
  }

  writeBytes(value: Uint8Array): void {
    this.writeU32(value.length);
    this.ensureCapacity(value.length);
    new Uint8Array(this.currentBuffer()).set(value, this.writePosition());
    this.offset += value.length;
  }

  writeOptional<T>(value: T | null, writeValue: (v: T) => void): void {
    if (value === null) {
      this.writeU8(0);
    } else {
      this.writeU8(1);
      writeValue(value);
    }
  }

  writeArray<T>(values: T[], writeElement: (v: T) => void): void {
    this.writeU32(values.length);
    for (const v of values) {
      writeElement(v);
    }
  }

  writeResult<T, E>(
    value: T | E | WireResult<T, E>,
    writeOk: (v: T) => void,
    writeErr: (e: E) => void
  ): void {
    if (
      typeof value === "object" &&
      value !== null &&
      "tag" in value &&
      value.tag === "ok" &&
      "value" in value
    ) {
      this.writeU8(0);
      writeOk(value.value as T);
      return;
    }
    if (
      typeof value === "object" &&
      value !== null &&
      "tag" in value &&
      value.tag === "err" &&
      "error" in value
    ) {
      this.writeU8(1);
      writeErr(value.error as E);
      return;
    }
    if (value instanceof Error) {
      this.writeU8(1);
      writeErr(value as E);
      return;
    }
    if (typeof value === "object" && value !== null) {
      throw new Error(
        "Ambiguous Result object. Pass wireOk(value) or wireErr(error) for object payloads."
      );
    }
    this.writeU8(0);
    writeOk(value as T);
  }
}

export function wireStringSize(value: string): number {
  return 4 + new TextEncoder().encode(value).length;
}

export interface WireCodec<T> {
  size(value: T): number;
  encode(writer: WireWriter, value: T): void;
  decode(reader: WireReader): T;
}
