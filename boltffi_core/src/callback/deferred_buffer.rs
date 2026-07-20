//! Ownership-transfer buffer for a deferred callback-vtable slot's encoded
//! parameter.
//!
//! A deferred slot (`boltffi_macros`'s `NativeCallbackMethodExpander::is_deferred`:
//! an async method, or a sync method with no return value) hands its caller
//! bytes the callee may read *after* the dispatching call has already
//! returned — a Rust-stack-scoped buffer would be a use-after-return by
//! then. [`transfer_deferred_callback_bytes`] leaks an owned buffer instead,
//! transferring ownership to whatever the vtable slot calls into.
//!
//! The paired [`boltffi_free_deferred_callback_bytes`] is the ONE dealloc
//! every consumer of such a buffer must call — Dart, JNI, a hand-written C
//! implementation, or any future backend. Allocation and deallocation both
//! go through Rust's own global allocator (via `Box<[u8]>`), so they always
//! agree regardless of platform: no raw libc `malloc`/`free` pairing, and
//! critically no foreign allocator (e.g. Dart's `package:ffi` `calloc`,
//! which is `CoTaskMemAlloc` on Windows, not the CRT heap) on either side.

/// Leaks `bytes`, returning a pointer and length the callee owns until it
/// passes both back to [`boltffi_free_deferred_callback_bytes`].
///
/// Allocation failure is handled by `Box`'s own machinery (Rust aborts via
/// `handle_alloc_error` rather than returning a null pointer with a stale
/// nonzero length) — there is no null-buffer case for a caller to check.
pub fn transfer_deferred_callback_bytes(bytes: Vec<u8>) -> (*mut u8, usize) {
    let len = bytes.len();
    let boxed: Box<[u8]> = bytes.into_boxed_slice();
    (Box::into_raw(boxed) as *mut u8, len)
}

/// Frees a buffer previously handed to a deferred callback-vtable slot by
/// [`transfer_deferred_callback_bytes`]. `len` must be exactly the length
/// that call returned — this reconstructs the same `Box<[u8]>` layout to
/// deallocate correctly.
#[unsafe(no_mangle)]
pub extern "C" fn boltffi_free_deferred_callback_bytes(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice_ptr = core::ptr::slice_from_raw_parts_mut(ptr, len);
    drop(unsafe { Box::from_raw(slice_ptr) });
}

/// Frees the encoded parameter buffer a **local** Rust implementation of a
/// callback trait receives through its vtable slot — the reverse direction
/// from the pair above: there Rust is the *caller* handing a buffer to a
/// foreign callee; here Rust is the *callee*, reading a buffer someone else
/// allocated (`boltffi_macros`' `local_handle.rs` codegen — a Rust value
/// wrapped as a `CallbackHandle` so it can be passed anywhere a foreign
/// callback is expected).
///
/// The two targets differ on who allocates that buffer, so they differ on
/// how to free it:
/// - Native: the only caller of a local vtable slot is this crate's own
///   generated `ForeignX` dispatch (`native.rs`'s `NativeCallbackMethodExpander`),
///   which always allocates via [`transfer_deferred_callback_bytes`] — so the
///   matching Rust-allocator dealloc applies here too. (Never a raw libc
///   `free`: that only "worked" by coincidence of native platforms' global
///   allocator routing through the system heap, and doesn't exist to import
///   on wasm32 at all — see the wasm32 branch below.)
/// - wasm32: the caller is JS, holding a numeric "local" handle via the
///   generated `wrapHandleFn` — it can only have written the buffer into wasm
///   linear memory through the exported `boltffi_wasm_alloc`, so the matching
///   `boltffi_wasm_free` dealloc applies instead.
#[cfg(not(target_arch = "wasm32"))]
pub fn free_local_callback_param_bytes(ptr: *mut u8, len: usize) {
    boltffi_free_deferred_callback_bytes(ptr, len);
}

#[cfg(target_arch = "wasm32")]
pub fn free_local_callback_param_bytes(ptr: *mut u8, len: usize) {
    crate::wasm::boltffi_wasm_free_impl(ptr as usize, len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_free_without_corrupting_bytes() {
        let (ptr, len) = transfer_deferred_callback_bytes(vec![1, 2, 3, 4]);
        assert_eq!(len, 4);
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        assert_eq!(bytes, &[1, 2, 3, 4]);
        boltffi_free_deferred_callback_bytes(ptr, len);
    }

    #[test]
    fn empty_buffer_round_trips() {
        let (ptr, len) = transfer_deferred_callback_bytes(Vec::new());
        assert_eq!(len, 0);
        boltffi_free_deferred_callback_bytes(ptr, len);
    }

    #[test]
    fn null_pointer_is_a_no_op() {
        boltffi_free_deferred_callback_bytes(core::ptr::null_mut(), 0);
    }

    #[test]
    fn local_callback_param_bytes_round_trip_through_the_transfer_buffer() {
        let (ptr, len) = transfer_deferred_callback_bytes(vec![9, 8, 7]);
        assert_eq!(len, 3);
        let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
        assert_eq!(bytes, &[9, 8, 7]);
        free_local_callback_param_bytes(ptr, len);
    }

    #[test]
    fn local_callback_param_bytes_null_pointer_is_a_no_op() {
        free_local_callback_param_bytes(core::ptr::null_mut(), 0);
    }
}
