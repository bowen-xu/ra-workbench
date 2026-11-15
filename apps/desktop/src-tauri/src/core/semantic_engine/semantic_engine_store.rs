use std::{
    env,
    path::{Path, PathBuf},
    ptr, slice,
    sync::Arc,
};

use libloading::Library;
use research_semantic_engine_api::{
    EngineEnvelope, EngineRequest, EngineResponse, FfiBuffer, OpenRequest,
    SEMANTIC_ENGINE_ABI_VERSION, SemanticEngineHandle,
};

use super::{
    SemanticNamespace, SemanticVectorRow, SemanticVectorSearchHit, SemanticVectorStore,
    StoredVectorMetadata, VectorError, VectorWriteReceipt,
};

type AbiVersionFn = unsafe extern "C" fn() -> u32;
type OpenFn =
    unsafe extern "C" fn(*const u8, usize, *mut SemanticEngineHandle, *mut FfiBuffer) -> i32;
type CallFn = unsafe extern "C" fn(SemanticEngineHandle, *const u8, usize, *mut FfiBuffer) -> i32;
type CloseFn = unsafe extern "C" fn(SemanticEngineHandle);
type FreeBufferFn = unsafe extern "C" fn(FfiBuffer);

struct EngineSymbols {
    open: OpenFn,
    call: CallFn,
    close: CloseFn,
    free_buffer: FreeBufferFn,
}

struct EngineSession {
    handle: SemanticEngineHandle,
    symbols: EngineSymbols,
    _library: Library,
}

// The ABI owns the handle until `close`, all calls are synchronous, and the engine's store is
// thread-safe. Keeping the `Library` in this struct guarantees the function pointers remain valid.
unsafe impl Send for EngineSession {}
unsafe impl Sync for EngineSession {}

impl Drop for EngineSession {
    fn drop(&mut self) {
        // SAFETY: this session exclusively owns the handle and closes it exactly once.
        unsafe { (self.symbols.close)(self.handle) };
    }
}

pub(crate) struct SemanticEngineVectorStore {
    session: Arc<EngineSession>,
}

impl SemanticEngineVectorStore {
    pub(super) async fn open(
        database_path: &Path,
        namespace: SemanticNamespace,
        dimension: usize,
    ) -> Result<Self, VectorError> {
        let library_path = required_engine_library_path()?;
        let database_path = database_path
            .to_str()
            .ok_or(VectorError::InvalidInput("索引目录不是有效 UTF-8"))?
            .to_owned();
        let session = tokio::task::spawn_blocking(move || {
            EngineSession::open(
                &library_path,
                OpenRequest {
                    schema_version: SEMANTIC_ENGINE_ABI_VERSION,
                    database_path,
                    namespace,
                    dimension,
                },
            )
        })
        .await
        .map_err(|_| VectorError::WorkerUnavailable)??;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    async fn request(&self, request: EngineRequest) -> Result<EngineResponse, VectorError> {
        let session = self.session.clone();
        tokio::task::spawn_blocking(move || session.call(request))
            .await
            .map_err(|_| VectorError::WorkerUnavailable)?
    }
}

impl SemanticVectorStore for SemanticEngineVectorStore {
    async fn ensure_ready(&self) -> Result<u64, VectorError> {
        match self.request(EngineRequest::EnsureReady).await? {
            EngineResponse::Ready(version) => Ok(version),
            _ => Err(unexpected_response()),
        }
    }

    async fn upsert(&self, row: &SemanticVectorRow) -> Result<VectorWriteReceipt, VectorError> {
        match self
            .request(EngineRequest::Upsert { row: row.clone() })
            .await?
        {
            EngineResponse::Upserted(receipt) => Ok(receipt),
            _ => Err(unexpected_response()),
        }
    }

    async fn contains(&self, row_id: &str, input_sha256: &str) -> Result<bool, VectorError> {
        match self
            .request(EngineRequest::Contains {
                row_id: row_id.to_owned(),
                input_sha256: input_sha256.to_owned(),
            })
            .await?
        {
            EngineResponse::Contains(contains) => Ok(contains),
            _ => Err(unexpected_response()),
        }
    }

    async fn list_metadata(&self) -> Result<Vec<StoredVectorMetadata>, VectorError> {
        match self.request(EngineRequest::ListMetadata).await? {
            EngineResponse::Metadata(metadata) => Ok(metadata),
            _ => Err(unexpected_response()),
        }
    }

    async fn search(
        &self,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticVectorSearchHit>, VectorError> {
        match self
            .request(EngineRequest::Search {
                query: query.to_vec(),
                limit,
            })
            .await?
        {
            EngineResponse::SearchHits(hits) => Ok(hits),
            _ => Err(unexpected_response()),
        }
    }

    async fn delete(&self, row_id: &str) -> Result<u64, VectorError> {
        match self
            .request(EngineRequest::Delete {
                row_id: row_id.to_owned(),
            })
            .await?
        {
            EngineResponse::Deleted(version) => Ok(version),
            _ => Err(unexpected_response()),
        }
    }
}

impl EngineSession {
    fn open(library_path: &Path, request: OpenRequest) -> Result<Self, VectorError> {
        let (library, symbols) = load_library(library_path)?;
        let request = serde_json::to_vec(&request)
            .map_err(|error| VectorError::EngineProtocol(error.to_string()))?;
        let mut handle = ptr::null_mut();
        let mut output = FfiBuffer::empty();
        // SAFETY: all pointers refer to live buffers for the duration of this synchronous call.
        let status =
            unsafe { (symbols.open)(request.as_ptr(), request.len(), &mut handle, &mut output) };
        let response = decode_output(output, symbols.free_buffer, status);
        match response {
            Ok(EngineResponse::Opened) if !handle.is_null() => Ok(Self {
                handle,
                symbols,
                _library: library,
            }),
            Ok(_) => {
                if !handle.is_null() {
                    // SAFETY: open returned this handle, but the protocol response was invalid.
                    unsafe { (symbols.close)(handle) };
                }
                Err(unexpected_response())
            }
            Err(error) => {
                if !handle.is_null() {
                    // SAFETY: open returned this handle, but decoding the response failed.
                    unsafe { (symbols.close)(handle) };
                }
                Err(error)
            }
        }
    }

    fn call(&self, request: EngineRequest) -> Result<EngineResponse, VectorError> {
        let request = serde_json::to_vec(&request)
            .map_err(|error| VectorError::EngineProtocol(error.to_string()))?;
        let mut output = FfiBuffer::empty();
        // SAFETY: the session owns a live handle and the request lives through the call.
        let status = unsafe {
            (self.symbols.call)(self.handle, request.as_ptr(), request.len(), &mut output)
        };
        decode_output(output, self.symbols.free_buffer, status)
    }
}

pub(crate) fn validate_required_engine() -> Result<(), VectorError> {
    let path = required_engine_library_path()?;
    let _ = load_library(&path)?;
    Ok(())
}

fn load_library(path: &Path) -> Result<(Library, EngineSymbols), VectorError> {
    if !path.is_file() {
        return Err(VectorError::EngineLoad(
            "dynamic library file is missing".to_owned(),
        ));
    }
    // SAFETY: the required symbols are checked immediately and the library remains loaded while
    // any copied function pointer can be called.
    let library = unsafe { Library::new(path) }
        .map_err(|error| VectorError::EngineLoad(error.to_string()))?;
    // SAFETY: symbol names and signatures are fixed by semantic-engine-api ABI version 1.
    let abi_version =
        required_symbol::<AbiVersionFn>(&library, b"research_semantic_engine_abi_version\0")?;
    // SAFETY: calling the version function requires no arguments or shared state.
    if unsafe { abi_version() } != SEMANTIC_ENGINE_ABI_VERSION {
        return Err(VectorError::EngineAbiMismatch);
    }
    let symbols = EngineSymbols {
        open: required_symbol(&library, b"research_semantic_engine_open\0")?,
        call: required_symbol(&library, b"research_semantic_engine_call\0")?,
        close: required_symbol(&library, b"research_semantic_engine_close\0")?,
        free_buffer: required_symbol(&library, b"research_semantic_engine_free_buffer\0")?,
    };
    Ok((library, symbols))
}

fn required_symbol<T>(library: &Library, name: &[u8]) -> Result<T, VectorError>
where
    T: Copy,
{
    // SAFETY: the caller chooses the function type from the validated ABI contract. The copied
    // pointer is only stored beside the live library that owns the symbol.
    unsafe { library.get::<T>(name) }
        .map(|symbol| *symbol)
        .map_err(|error| VectorError::EngineLoad(error.to_string()))
}

fn decode_output(
    output: FfiBuffer,
    free_buffer: FreeBufferFn,
    status: i32,
) -> Result<EngineResponse, VectorError> {
    if output.data.is_null() || output.length == 0 || output.length > output.capacity {
        return Err(VectorError::EngineProtocol(
            "engine returned an invalid buffer".to_owned(),
        ));
    }
    // SAFETY: the engine allocated this readable buffer and transfers it until `free_buffer`.
    let bytes = unsafe { slice::from_raw_parts(output.data, output.length) }.to_vec();
    // SAFETY: the same engine allocated the buffer and accepts its exact returned parts.
    unsafe { free_buffer(output) };
    let envelope: EngineEnvelope = serde_json::from_slice(&bytes)
        .map_err(|error| VectorError::EngineProtocol(error.to_string()))?;
    match (status, envelope) {
        (0, EngineEnvelope::Ok(response)) => Ok(response),
        (_, EngineEnvelope::Error(error)) => Err(VectorError::EngineOperation { code: error.code }),
        _ => Err(VectorError::EngineProtocol(
            "engine status and response disagree".to_owned(),
        )),
    }
}

fn unexpected_response() -> VectorError {
    VectorError::EngineProtocol("engine returned an unexpected response".to_owned())
}

fn required_engine_library_path() -> Result<PathBuf, VectorError> {
    let executable =
        env::current_exe().map_err(|error| VectorError::EngineLoad(error.to_string()))?;
    if let Some(path) = bundled_engine_library_path(&executable) {
        return Ok(path);
    }
    if let Some(path) =
        env::var_os("RESEARCH_SEMANTIC_ENGINE_LIBRARY").filter(|path| !path.is_empty())
    {
        return Ok(PathBuf::from(path));
    }
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".generated")
        .join("semantic-engine")
        .join(engine_library_filename()))
}

#[cfg(target_os = "macos")]
fn bundled_engine_library_path(executable: &Path) -> Option<PathBuf> {
    let mac_os_directory = executable.parent()?;
    if mac_os_directory.file_name()?.to_str()? != "MacOS" {
        return None;
    }
    let contents_directory = mac_os_directory.parent()?;
    if contents_directory.file_name()?.to_str()? != "Contents"
        || contents_directory.parent()?.extension()?.to_str()? != "app"
    {
        return None;
    }
    Some(
        contents_directory
            .join("Frameworks")
            .join(engine_library_filename()),
    )
}

#[cfg(not(target_os = "macos"))]
fn bundled_engine_library_path(_executable: &Path) -> Option<PathBuf> {
    None
}

const fn engine_library_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "libresearch_semantic_engine.dylib"
    } else if cfg!(target_os = "windows") {
        "research_semantic_engine.dll"
    } else {
        "libresearch_semantic_engine.so"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn packaged_debug_app_uses_its_bundled_engine() {
        let executable = Path::new(
            "/workspace/.tmp/Research Assistant Dev.app/Contents/MacOS/research-assistant",
        );

        assert_eq!(
            bundled_engine_library_path(executable),
            Some(PathBuf::from(
                "/workspace/.tmp/Research Assistant Dev.app/Contents/Frameworks/\
                 libresearch_semantic_engine.dylib"
            ))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unbundled_debug_executable_does_not_claim_a_frameworks_path() {
        let executable =
            Path::new("/workspace/apps/desktop/src-tauri/target/debug/research-assistant");

        assert_eq!(bundled_engine_library_path(executable), None);
    }

    #[test]
    fn missing_required_library_is_a_hard_error() {
        let directory = tempfile::tempdir().unwrap();
        let error = match load_library(&directory.path().join(engine_library_filename())) {
            Ok(_) => panic!("a missing required engine must fail"),
            Err(error) => error,
        };

        assert!(matches!(error, VectorError::EngineLoad(_)));
    }
}
