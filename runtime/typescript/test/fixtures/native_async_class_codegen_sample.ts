// A literal transcription of class.txt's native_async branch output for a class method (the
// react-native track's stage 3 gap: native_async_codegen_sample.ts (stage 2) proved the
// free-function shape type-checks against the real runtime types, but ParseClient IS a class --
// this closes the same loop for TsClassMethodMode::Async's native_async arm). The method body
// below is the exact output of `ClassTemplate::render()` for the fixture
// `templates.rs`'s `class_native_async_scalar_return_dispatches_through_poll_async_native` test
// asserts as substrings; the surrounding class is trimmed to just the handle field + one method
// (finalizer/dispose scaffolding is an orthogonal, already-proven class-rendering concern with
// its own DOM/lib requirements, not part of native_async).
// Not wired into any build; checked ad hoc via `npm run test:codegen-sample`.
import type { NativeBoltFFIModule } from "../../src/native.js";

interface CounterExports {
  boltffi_method_class_counter_delayed_add: (handle: number) => bigint;
  boltffi_async_method_class_counter_delayed_add_poll: (
    h: bigint,
    callbackData: bigint,
    callback: unknown
  ) => void;
  boltffi_async_method_class_counter_delayed_add_complete: (
    h: bigint,
    statusBuf: Uint8Array
  ) => number;
  boltffi_async_method_class_counter_delayed_add_free: (h: bigint) => void;
}

declare const _module: NativeBoltFFIModule<CounterExports>;
declare const _exports: CounterExports;

export class Counter {
  private _handle: number;

  private constructor(handle: number) {
    this._handle = handle;
  }

  async delayedAdd(): Promise<number> {
    const handle = (_exports.boltffi_method_class_counter_delayed_add as Function)(this._handle);
    const awaitedHandle = await _module.asyncManager.pollAsyncNative(
      handle,
      (h: bigint, callbackData: bigint, callback: unknown) =>
        (_exports.boltffi_async_method_class_counter_delayed_add_poll as Function)(h, callbackData, callback)
    );
    try {
      const statusBuf = new Uint8Array(4);
      const result = (_exports.boltffi_async_method_class_counter_delayed_add_complete as Function)(
        awaitedHandle,
        statusBuf
      ) as number;
      const statusCode = new DataView(statusBuf.buffer).getInt32(0, true);
      if (statusCode !== 0) {
        throw new Error(`native async call failed with status ${statusCode}`);
      }
      return result;
    } finally {
      (_exports.boltffi_async_method_class_counter_delayed_add_free as Function)(awaitedHandle);
    }
  }
}
