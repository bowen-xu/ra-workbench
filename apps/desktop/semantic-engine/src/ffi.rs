use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    ptr, slice,
};

use research_semantic_engine_api::{
    EngineEnvelope, EngineErrorResponse, EngineRequest, EngineResponse, FfiBuffer, OpenRequest,
    SEMANTIC_ENGINE_ABI_VERSION, SemanticEngineHandle,
};
use tokio::runtime::Builder;

use crate::{error::EngineError, store::LanceDbStore};

struct EngineHandle {
    store: LanceDbStore,
}

#[unsafe(no_mangle)]
pub extern "C" fn research_semantic_engine_abi_version() -> u32 {
    SEMANTIC_ENGINE_ABI_VERSION
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn research_semantic_engine_open(
    request_data: *const u8,
    request_length: usize,
    output_handle: *mut SemanticEngineHandle,
    output: *mut FfiBuffer,
) -> i32 {
    ffi_boundary(output, || {
        if output_handle.is_null() {
            return Err(EngineError::InvalidInput("output handle pointer is null"));
        }
        let request: OpenRequest = decode_json(request_data, request_length)?;
        if request.schema_version != SEMANTIC_ENGINE_ABI_VERSION {
            return Err(EngineError::IncompatibleRequest);
        }
        let runtime = Builder::new_multi_thread().enable_all().build()?;
        let store = runtime.block_on(LanceDbStore::open(
            Path::new(&request.database_path),
            request.namespace,
            request.dimension,
        ))?;
        let handle = Box::into_raw(Box::new(EngineHandle { store })).cast::<c_void>();
        // SAFETY: the caller provided a non-null pointer for one handle value.
        unsafe { ptr::write(output_handle, handle) };
        Ok(EngineResponse::Opened)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn research_semantic_engine_call(
    handle: SemanticEngineHandle,
    request_data: *const u8,
    request_length: usize,
    output: *mut FfiBuffer,
) -> i32 {
    ffi_boundary(output, || {
        if handle.is_null() {
            return Err(EngineError::InvalidInput("semantic engine handle is null"));
        }
        let request: EngineRequest = decode_json(request_data, request_length)?;
        // SAFETY: handles are created by `research_semantic_engine_open` and remain owned by
        // the caller until exactly one close operation.
        let engine = unsafe { &*(handle.cast::<EngineHandle>()) };
        let runtime = Builder::new_multi_thread().enable_all().build()?;
        runtime.block_on(execute(&engine.store, request))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn research_semantic_engine_close(handle: SemanticEngineHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the handle came from `Box::into_raw` in open and close consumes it once.
        drop(unsafe { Box::from_raw(handle.cast::<EngineHandle>()) });
    }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn research_semantic_engine_free_buffer(buffer: FfiBuffer) {
    if buffer.data.is_null() || buffer.capacity == 0 {
        return;
    }
    // SAFETY: every non-empty output buffer is created from a Vec with these exact parts.
    drop(unsafe { Vec::from_raw_parts(buffer.data, buffer.length, buffer.capacity) });
}

async fn execute(
    store: &LanceDbStore,
    request: EngineRequest,
) -> Result<EngineResponse, EngineError> {
    match request {
        EngineRequest::EnsureReady => Ok(EngineResponse::Ready(store.ensure_ready().await?)),
        EngineRequest::Upsert { row } => Ok(EngineResponse::Upserted(store.upsert(&row).await?)),
        EngineRequest::Contains {
            row_id,
            input_sha256,
        } => Ok(EngineResponse::Contains(
            store.contains(&row_id, &input_sha256).await?,
        )),
        EngineRequest::ListMetadata => Ok(EngineResponse::Metadata(store.list_metadata().await?)),
        EngineRequest::Search { query, limit } => Ok(EngineResponse::SearchHits(
            store.search(&query, limit).await?,
        )),
        EngineRequest::Delete { row_id } => {
            Ok(EngineResponse::Deleted(store.delete(&row_id).await?))
        }
    }
}

fn ffi_boundary<F>(output: *mut FfiBuffer, operation: F) -> i32
where
    F: FnOnce() -> Result<EngineResponse, EngineError>,
{
    if output.is_null() {
        return 1;
    }
    let result = catch_unwind(AssertUnwindSafe(operation));
    let (status, envelope) = match result {
        Ok(Ok(response)) => (0, EngineEnvelope::Ok(response)),
        Ok(Err(error)) => (1, EngineEnvelope::Error(error.response())),
        Err(_) => (
            1,
            EngineEnvelope::Error(EngineErrorResponse {
                code: "panic".to_owned(),
                message: "semantic engine trapped an internal panic".to_owned(),
            }),
        ),
    };
    let buffer = serde_json::to_vec(&envelope)
        .map(buffer_from_vec)
        .unwrap_or_else(|_| FfiBuffer::empty());
    // SAFETY: the caller provided a non-null pointer for one FfiBuffer value.
    unsafe { ptr::write(output, buffer) };
    status
}

fn decode_json<T>(data: *const u8, length: usize) -> Result<T, EngineError>
where
    T: serde::de::DeserializeOwned,
{
    if data.is_null() || length == 0 || length > 128 * 1024 * 1024 {
        return Err(EngineError::InvalidInput("JSON request buffer is invalid"));
    }
    // SAFETY: the caller guarantees that the request buffer is readable for `length` bytes
    // during this synchronous call. The bytes are deserialized before returning.
    let bytes = unsafe { slice::from_raw_parts(data, length) };
    Ok(serde_json::from_slice(bytes)?)
}

fn buffer_from_vec(mut bytes: Vec<u8>) -> FfiBuffer {
    let buffer = FfiBuffer {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        capacity: bytes.capacity(),
    };
    std::mem::forget(bytes);
    buffer
}
