//! Narrow C ABI over the versioned protocol.
//!
//! The ABI is process-wide, synchronized, and non-reentrant. Handles and buffer tokens are
//! monotonically allocated. A returned buffer remains owned by Narrata until exactly one
//! successful `nar_buffer_free` call. No Rust enum, collection, borrow, or trait crosses the ABI.

#![deny(unsafe_op_in_unsafe_fn)]

use std::{
    collections::BTreeMap,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
    sync::{Mutex, MutexGuard, OnceLock},
};

use narrata_protocol::{
    PROTOCOL_ABI_VERSION, PROTOCOL_VERSION, ProtocolEngine,
    dto::{Diagnostic, Response, response},
};
use prost::Message;

#[repr(i32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NarStatus {
    Ok = 0,
    InvalidArgument = 1,
    InvalidHandle = 2,
    ProtocolError = 3,
    Panic = 4,
    Internal = 5,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NarBuffer {
    pub data: *const u8,
    pub len: usize,
    pub token: u64,
}

impl NarBuffer {
    const EMPTY: Self = Self {
        data: ptr::null(),
        len: 0,
        token: 0,
    };
}

#[derive(Default)]
struct Registry {
    engines: BTreeMap<u64, ProtocolEngine>,
    buffers: BTreeMap<u64, Box<[u8]>>,
    next_engine: u64,
    next_buffer: u64,
}

impl Registry {
    fn allocate_engine(&mut self) -> Result<u64, NarStatus> {
        let handle = next_token(&mut self.next_engine)?;
        self.engines.insert(handle, ProtocolEngine::default());
        Ok(handle)
    }

    fn allocate_buffer(&mut self, bytes: Vec<u8>) -> Result<NarBuffer, NarStatus> {
        let token = next_token(&mut self.next_buffer)?;
        let bytes = bytes.into_boxed_slice();
        let buffer = NarBuffer {
            data: bytes.as_ptr(),
            len: bytes.len(),
            token,
        };
        self.buffers.insert(token, bytes);
        Ok(buffer)
    }
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn next_token(next: &mut u64) -> Result<u64, NarStatus> {
    let value = if *next == 0 { 1 } else { *next };
    *next = value.checked_add(1).ok_or(NarStatus::Internal)?;
    Ok(value)
}

#[unsafe(no_mangle)]
pub extern "C" fn nar_abi_version() -> u32 {
    PROTOCOL_ABI_VERSION
}

/// # Safety
/// `out_handle` must be non-null, aligned, and writable for one `u64`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nar_engine_create(out_handle: *mut u64) -> NarStatus {
    boundary(|| {
        if out_handle.is_null() {
            return NarStatus::InvalidArgument;
        }
        let handle = match registry().allocate_engine() {
            Ok(value) => value,
            Err(status) => return status,
        };
        // SAFETY: The caller contract above requires a valid writable pointer.
        unsafe { out_handle.write(handle) };
        NarStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn nar_engine_free(handle: u64) -> NarStatus {
    boundary(|| {
        if registry().engines.remove(&handle).is_some() {
            NarStatus::Ok
        } else {
            NarStatus::InvalidHandle
        }
    })
}

/// # Safety
/// For non-zero `input_len`, `input` must point to that many readable bytes. `out_buffer` must be
/// non-null, aligned, and writable for one `NarBuffer`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nar_engine_call(
    handle: u64,
    input: *const u8,
    input_len: usize,
    out_buffer: *mut NarBuffer,
) -> NarStatus {
    if out_buffer.is_null() || (input_len != 0 && input.is_null()) {
        return NarStatus::InvalidArgument;
    }
    // SAFETY: The caller contract requires a valid writable output pointer.
    unsafe { out_buffer.write(NarBuffer::EMPTY) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The caller contract requires `input_len` readable bytes when non-zero. A null
        // pointer is replaced by a non-null dangling pointer for the zero-length slice.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                if input_len == 0 {
                    std::ptr::NonNull::<u8>::dangling().as_ptr().cast_const()
                } else {
                    input
                },
                input_len,
            )
        };
        let response = {
            let mut registry = registry();
            let Some(engine) = registry.engines.get_mut(&handle) else {
                return Err(NarStatus::InvalidHandle);
            };
            engine
                .handle_bytes(bytes)
                .map_err(|_| NarStatus::ProtocolError)?
        };
        let buffer = registry().allocate_buffer(response)?;
        // SAFETY: The caller contract requires a valid writable output pointer.
        unsafe { out_buffer.write(buffer) };
        Ok(NarStatus::Ok)
    }));
    match result {
        Ok(Ok(status)) => status,
        Ok(Err(status)) => status,
        Err(_) => {
            let bytes = panic_diagnostic();
            if let Ok(buffer) = registry().allocate_buffer(bytes) {
                // SAFETY: The caller contract requires a valid writable output pointer.
                unsafe { out_buffer.write(buffer) };
            }
            NarStatus::Panic
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nar_buffer_free(token: u64) -> NarStatus {
    boundary(|| {
        if token == 0 || registry().buffers.remove(&token).is_none() {
            NarStatus::InvalidHandle
        } else {
            NarStatus::Ok
        }
    })
}

fn panic_diagnostic() -> Vec<u8> {
    Response {
        protocol_version: PROTOCOL_VERSION,
        request_id: 0,
        body: Some(response::Body::Diagnostic(Diagnostic {
            code: "NAR-F0001".to_owned(),
            class: "panic".to_owned(),
            message: "a panic was contained at the C ABI boundary".to_owned(),
            retryable: false,
        })),
    }
    .encode_to_vec()
}

fn boundary(operation: impl FnOnce() -> NarStatus) -> NarStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(NarStatus::Panic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_is_contained() {
        assert_eq!(boundary(|| panic!("boundary test")), NarStatus::Panic);
    }
}
