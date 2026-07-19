// A literal transcription of what async_function.txt's native_async branch emits for an
// async_scalar-returning method (the exact shape boltffi_bindgen/src/render/typescript/
// templates.rs's native_async_scalar_return_dispatches_through_poll_async_native test asserts
// string-contains on) -- proves the emitted call shape actually TYPE-CHECKS against the real
// @boltffi/runtime types, not just that the right substrings appear in the rendered text.
// Not wired into any build; checked ad hoc via
// `npx tsc --noEmit --module esnext --moduleResolution bundler --target es2020 --strict
// test/fixtures/native_async_codegen_sample.ts` (docs/tracks/react-native.md stage 2 evidence).
import type { NativeBoltFFIModule } from "../../src/native.js";

interface DelayedAddExports {
  boltffi_method_class_counter_delayed_add: (handle: bigint, amount: number) => bigint;
  boltffi_async_method_class_counter_delayed_add_poll: (
    h: bigint,
    callbackData: bigint,
    callback: unknown
  ) => void;
  boltffi_async_method_class_counter_delayed_add_complete: (h: bigint, statusBuf: Uint8Array) => number;
  boltffi_async_method_class_counter_delayed_add_free: (h: bigint) => void;
}

declare const _module: NativeBoltFFIModule<DelayedAddExports>;
declare const _exports: DelayedAddExports;

export async function delayedAdd(handle: bigint, amount: number): Promise<number> {
  const h = (_exports.boltffi_method_class_counter_delayed_add as Function)(handle, amount);
  const awaitedHandle = await _module.asyncManager.pollAsyncNative(
    h,
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
