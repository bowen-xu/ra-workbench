//! One request's private input file, independent of the executable's retention.
use crate::runtime_plugins::native::{NativeHostError, NativeProfile, NativeResult};
use research_workbench_native_sdk::literature_trace_protocol::{
    Request, SnapshotInput, SnapshotRef,
};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
pub(crate) struct InputCleanup {
    failed: AtomicBool,
    cause: Mutex<Option<io::ErrorKind>>,
}

impl InputCleanup {
    pub(crate) fn record(&self, cause: io::ErrorKind) {
        if let Ok(mut slot) = self.cause.lock() {
            *slot = Some(cause);
        }
        self.failed.store(true, Ordering::Release);
    }
    pub(crate) fn failure(&self) -> Option<String> {
        if !self.failed.load(Ordering::Acquire) {
            return None;
        }
        let cause = self
            .cause
            .lock()
            .ok()
            .and_then(|slot| *slot)
            .unwrap_or(io::ErrorKind::Other);
        Some(format!("原生输入清理失败：{cause:?}"))
    }
}

pub(crate) struct InputLease {
    path: Option<PathBuf>,
    result_path: Option<PathBuf>,
    cleanup: Arc<InputCleanup>,
}

#[cfg(unix)]
struct PayloadIdentity<'a> {
    profile: NativeProfile,
    package_key: &'a str,
    instance_id: &'a str,
    request_id: &'a str,
    input_handle: String,
    result_handle: Option<String>,
}

impl InputLease {
    /// Run on a blocking worker after every database/admission guard is dropped.
    /// The directory is the Core-owned input cwd, never a plugin-chosen path.
    #[cfg(unix)]
    pub(crate) fn stage(
        directory: &Path,
        request: &Request,
        input: &SnapshotInput,
        cleanup: Arc<InputCleanup>,
    ) -> NativeResult<(Self, SnapshotRef)> {
        Self::stage_literature(directory, request, input, PayloadKind::Trace, cleanup)
    }

    #[cfg(unix)]
    pub(crate) fn stage_catalog(
        directory: &Path,
        request: &Request,
        input: &research_workbench_native_sdk::catalog::CatalogInput,
        cleanup: Arc<InputCleanup>,
    ) -> NativeResult<(Self, SnapshotRef)> {
        Self::stage_literature(directory, request, input, PayloadKind::Catalog, cleanup)
    }

    #[cfg(unix)]
    fn stage_literature(
        directory: &Path,
        request: &Request,
        input: &impl serde::Serialize,
        kind: PayloadKind,
        cleanup: Arc<InputCleanup>,
    ) -> NativeResult<(Self, SnapshotRef)> {
        use research_workbench_native_sdk::literature_trace_protocol as protocol;
        let identity = PayloadIdentity {
            profile: NativeProfile::NoteTraceDecision,
            package_key: &request.package_key,
            instance_id: &request.instance_id,
            request_id: &request.request_id,
            input_handle: match kind {
                PayloadKind::Trace => protocol::snapshot_handle(&request.request_id),
                PayloadKind::Catalog => protocol::catalog_input_handle(&request.request_id),
            },
            result_handle: match kind {
                PayloadKind::Trace => None,
                PayloadKind::Catalog => Some(protocol::catalog_result_handle(&request.request_id)),
            },
        };
        Self::stage_value(directory, identity, input, cleanup)
    }

    #[cfg(unix)]
    pub(crate) fn stage_projects(
        directory: &Path,
        request: &research_workbench_native_sdk::writing_projects_protocol::Request,
        input: &research_workbench_native_sdk::writing_project::Input,
        cleanup: Arc<InputCleanup>,
    ) -> NativeResult<(Self, SnapshotRef)> {
        use research_workbench_native_sdk::writing_projects_protocol as protocol;
        let identity = PayloadIdentity {
            profile: NativeProfile::WritingProjects,
            package_key: &request.package_key,
            instance_id: &request.instance_id,
            request_id: &request.request_id,
            input_handle: protocol::input_handle(&request.request_id),
            result_handle: Some(protocol::result_handle(&request.request_id)),
        };
        Self::stage_value(directory, identity, input, cleanup)
    }

    #[cfg(unix)]
    fn stage_value(
        directory: &Path,
        identity: PayloadIdentity<'_>,
        input: &impl serde::Serialize,
        cleanup: Arc<InputCleanup>,
    ) -> NativeResult<(Self, SnapshotRef)> {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let io_error = |kind| {
            NativeHostError::for_profile(
                identity.profile,
                "plugin_host_snapshot_stage_failed",
                format!("原生输入准备失败：{kind:?}"),
            )
            .package(identity.package_key)
        };
        let path = directory.join(&identity.input_handle);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| io_error(error.kind()))?;
        let lease = Self {
            path: Some(path),
            result_path: identity
                .result_handle
                .as_ref()
                .map(|handle| directory.join(handle)),
            cleanup,
        };
        use research_workbench_native_sdk::streaming_payload::{self, PayloadError};
        let (file, digest) = streaming_payload::write_json(file, input).map_err(|error| {
            let kind = match error {
                PayloadError::Serialization(kind) | PayloadError::Io(kind) => kind,
                _ => io::ErrorKind::InvalidData,
            };
            io_error(kind)
        })?;
        file.sync_all().map_err(|error| io_error(error.kind()))?;
        file.set_permissions(fs::Permissions::from_mode(0o400))
            .map_err(|error| io_error(error.kind()))?;
        let reference = SnapshotRef {
            relative_handle: identity.input_handle,
            byte_size: digest.byte_size,
            sha256: digest.sha256,
            package_key: identity.package_key.into(),
            instance_id: identity.instance_id.into(),
            request_id: identity.request_id.into(),
        };
        drop(file);
        Ok((lease, reference))
    }

    pub(crate) fn clear(&mut self) -> Result<(), io::ErrorKind> {
        let mut failure = None;
        for slot in [&mut self.path, &mut self.result_path] {
            if let Some(path) = slot.take()
                && let Err(error) = fs::remove_file(&path)
                && error.kind() != io::ErrorKind::NotFound
            {
                let cause = error.kind();
                *slot = Some(path);
                self.cleanup.record(cause);
                failure = Some(cause);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
enum PayloadKind {
    Trace,
    Catalog,
}

impl Drop for InputLease {
    fn drop(&mut self) {
        let _ = self.clear();
    }
}

#[cfg(test)]
mod tests;
