//! The existing process owner supplies spawn, explicit environment, dual pipes,
//! group termination and reap. This adapter only adds the frozen plugin wire.
mod catalog;
mod exchange;
mod inputs;
mod projects;
use super::{
    NativeHostError, NativePreviewError, NativeProfile, NativeResult,
    diagnostics::NativeTerminationDiagnostic,
    package::StagedHost,
    wire::{WireRequest, WireResponse},
};
use crate::agent::process::{OutputStream, PipeInput, PipeProcess, ProcessSpec};
use crate::runtime_plugins::literature_trace::input::{InputCleanup, InputLease};
use research_agent_core::mcp::FrameDecoder;
use research_workbench_native_sdk::catalog::{CatalogInput, CatalogOutput};
use research_workbench_native_sdk::literature_trace_protocol as literature;
use research_workbench_native_sdk::protocol::{
    self, PreviewErrorCode, PreviewFailure, Request, RequestBody, ResponseBody,
};
use research_workbench_native_sdk::{
    writing_project, writing_projects_protocol as projects_protocol,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::time::timeout;

const DRAIN_WAIT: Duration = Duration::from_secs(2);
const STDERR_BUDGET: usize = 16 * 1024;

enum DrainMode {
    ProtocolTail,
    Cleanup,
}

pub(super) enum NativeInput {
    Trace(literature::SnapshotInput),
    Catalog(CatalogInput),
    Projects(writing_project::Input),
}

pub(crate) struct NativeHost {
    process: PipeProcess,
    input: PipeInput,
    decoder: FrameDecoder,
    staged: Option<StagedHost>,
    package_key: String,
    instance_id: String,
    last_request_id: Option<String>,
    stderr_bytes: usize,
    failed: bool,
    profile: NativeProfile,
    input_cleanup: Arc<InputCleanup>,
    input_lease: Option<InputLease>,
    input_staging:
        Option<tokio::task::JoinHandle<NativeResult<(InputLease, literature::SnapshotRef)>>>,
    output_reading: Option<tokio::task::JoinHandle<NativeResult<WireResponse>>>,
    payload_inflight: bool,
    // Inject only the initial signal result. The actual owned process, drain,
    // pending group and input cleanup still run in the regression tests.
    #[cfg(test)]
    initial_termination_error: Option<crate::agent::process::ProcessError>,
    #[cfg(all(test, target_os = "macos"))]
    cleanup_pause: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}

impl NativeHost {
    #[cfg(test)]
    pub(crate) fn process_id_for_test(&self) -> u32 {
        self.process.process_id()
    }

    pub(super) fn spawn(staged: StagedHost, key: &str) -> NativeResult<Self> {
        Self::spawn_for(staged, key, NativeProfile::WritingPreview)
    }
    pub(crate) fn spawn_for(
        staged: StagedHost,
        key: &str,
        profile: NativeProfile,
    ) -> NativeResult<Self> {
        if staged.profile != profile {
            return Err(NativeHostError::for_profile(
                profile,
                "plugin_host_profile_unsupported",
                "原生插件资源不属于所选执行协议。",
            )
            .package(key));
        }
        let spec = ProcessSpec {
            program: staged.executable.clone(),
            arguments: if profile == NativeProfile::WritingProjects {
                vec!["--project-files-v2".into()]
            } else {
                Vec::new()
            },
            directory: staged
                .input_directory
                .as_ref()
                .unwrap_or(&staged.directory)
                .path()
                .to_owned(),
            environment: BTreeMap::new(),
        };
        let (process, input) = PipeProcess::spawn(spec).map_err(|error| {
            NativeHostError::for_profile(profile, "plugin_host_spawn_failed", error.to_string())
                .package(key)
        })?;
        Ok(Self {
            process,
            input,
            decoder: FrameDecoder::with_max_frame_bytes(profile.max_frame_bytes()),
            staged: Some(staged),
            package_key: key.into(),
            instance_id: uuid::Uuid::new_v4().to_string(),
            last_request_id: None,
            stderr_bytes: 0,
            failed: false,
            profile,
            input_cleanup: Arc::default(),
            input_lease: None,
            input_staging: None,
            output_reading: None,
            payload_inflight: false,
            #[cfg(test)]
            initial_termination_error: None,
            #[cfg(all(test, target_os = "macos"))]
            cleanup_pause: None,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }
    pub(super) fn profile(&self) -> NativeProfile {
        self.profile
    }
    pub(crate) fn is_failed(&self) -> bool {
        self.failed || self.input_cleanup.failure().is_some()
    }
    /// The last actual exchange identity, never a Core submission receipt.
    pub(crate) fn request_identity(&self) -> Option<(String, String)> {
        Some((
            self.instance_id.clone(),
            self.last_request_id.as_ref()?.clone(),
        ))
    }

    pub(crate) async fn initialize(&mut self) -> NativeResult<()> {
        let request = match self.profile {
            NativeProfile::WritingPreview => self.writing_request(RequestBody::Initialize),
            NativeProfile::WritingProjects => {
                self.project_request(projects_protocol::RequestBody::Initialize {})
            }
            NativeProfile::NoteTraceDecision => {
                self.literature_request(literature::RequestBody::Initialize)
            }
        };
        match self
            .exchange(request, None, Duration::from_secs(10))
            .await?
        {
            WireResponse::Writing(ResponseBody::Initialized { .. })
            | WireResponse::Literature(literature::ResponseBody::Initialized { .. })
            | WireResponse::ProjectBody(projects_protocol::ResponseBody::Initialized { .. }) => {
                Ok(())
            }
            _ => Err(self.error("plugin_host_protocol_invalid", "原生插件未确认初始化。")),
        }
    }

    fn poison(&mut self) {
        self.failed = true;
        self.input.close();
        // The result is collected during bounded cleanup, not claimed here.
        let _ = self.process.terminate();
    }

    pub(crate) fn error(&self, code: &str, message: impl Into<String>) -> Box<NativePreviewError> {
        let mut error =
            NativeHostError::for_profile(self.profile, code, message).package(&self.package_key);
        error.instance_id = Some(self.instance_id.clone());
        error.request_id = self.last_request_id.clone();
        if let Some(exit) = self.process.exit() {
            error.exit_code = exit.code;
            error.signal = exit.signal;
        }
        error
    }

    pub(super) fn domain_error(&self, failure: PreviewFailure) -> Box<NativePreviewError> {
        let code = match failure.code {
            PreviewErrorCode::InvalidInput => "invalid_input",
            PreviewErrorCode::UnsafePath => "unsafe_path",
            PreviewErrorCode::FileTooLarge => "file_too_large",
            PreviewErrorCode::StorageError => "storage_error",
        };
        self.error(code, failure.message)
    }

    fn writing_request(&self, body: RequestBody) -> WireRequest {
        WireRequest::Writing(Request {
            protocol_version: protocol::PROTOCOL_VERSION,
            plugin_id: protocol::PLUGIN_ID.into(),
            package_key: self.package_key.clone(),
            instance_id: self.instance_id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            body,
        })
    }

    fn literature_request(&self, body: literature::RequestBody) -> WireRequest {
        WireRequest::Literature(literature::Request {
            protocol_version: literature::PROTOCOL_VERSION,
            plugin_id: literature::PLUGIN_ID.into(),
            package_key: self.package_key.clone(),
            instance_id: self.instance_id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            body,
        })
    }

    fn project_request(&self, body: projects_protocol::RequestBody) -> WireRequest {
        WireRequest::Projects(projects_protocol::Request {
            protocol_version: projects_protocol::PROTOCOL_VERSION,
            plugin_id: projects_protocol::PLUGIN_ID.into(),
            package_key: self.package_key.clone(),
            instance_id: self.instance_id.clone(),
            request_id: uuid::Uuid::new_v4().to_string(),
            body,
        })
    }

    /// Preserve the original finite writing call; it cannot route into v2.
    pub(super) async fn request(
        &mut self,
        body: RequestBody,
        wait: Duration,
    ) -> NativeResult<ResponseBody> {
        if self.profile != NativeProfile::WritingPreview {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "当前原生实例不接受写作协议。",
            ));
        }
        let request = self.writing_request(body);
        match self.exchange(request, None, wait).await? {
            WireResponse::Writing(body) => Ok(body),
            _ => Err(self.error("plugin_host_protocol_invalid", "原生插件回复协议不匹配。")),
        }
    }

    pub(crate) async fn decide_note_trace(
        &mut self,
        input: literature::SnapshotInput,
        wait: Duration,
    ) -> NativeResult<literature::TraceDecision> {
        if self.profile != NativeProfile::NoteTraceDecision {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "当前原生实例不接受文献协议。",
            ));
        }
        let request = self.literature_request(literature::RequestBody::Initialize);
        match self
            .exchange(request, Some(NativeInput::Trace(input)), wait)
            .await?
        {
            WireResponse::Literature(literature::ResponseBody::NoteTraceDecision { decision }) => {
                Ok(decision)
            }
            _ => Err(self.error(
                "plugin_host_protocol_invalid",
                "原生插件回复种类与文献决定不匹配。",
            )),
        }
    }

    async fn drain(&mut self, mode: DrainMode) -> NativeResult<()> {
        let strict_protocol_tail = matches!(mode, DrainMode::ProtocolTail);
        match timeout(DRAIN_WAIT, async {
            while let Some(chunk) = self
                .process
                .next_output()
                .await
                .map_err(|error| self.error("plugin_host_cleanup_failed", error.to_string()))?
            {
                if strict_protocol_tail && chunk.stream == OutputStream::Stdout {
                    return Err(self.error(
                        "plugin_host_protocol_invalid",
                        "原生插件在关闭确认后发送了额外输出。",
                    ));
                }
                if strict_protocol_tail && chunk.stream == OutputStream::Stderr {
                    self.stderr_bytes = self.stderr_bytes.saturating_add(chunk.bytes.len());
                    if self.stderr_bytes > STDERR_BUDGET {
                        return Err(self.error(
                            "plugin_host_output_limit",
                            "原生插件超出诊断输出预算；原始输出未保存。",
                        ));
                    }
                }
            }
            Ok::<(), Box<NativePreviewError>>(())
        })
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(self.error(
                "plugin_host_cleanup_timeout",
                "原生插件尚未完成排空／回收；不能确认收尾成功。",
            )),
        }
    }

    pub(crate) async fn finish_failed(&mut self, error: &mut NativePreviewError) {
        self.failed = true;
        self.input.close();
        self.finish_input().await;
        if let Some(cause) = self.clear_input() {
            error.add_cleanup(cause);
        }
        let had_cached_exit = self.process.exit().is_some();
        let termination_error = self.terminate_failed().err();
        let diagnostic = termination_error.as_ref().and_then(|cause| {
            NativeTerminationDiagnostic::from_initial_group_termination(cause, had_cached_exit)
        });
        // Success here means both EOFs, actual root reap AND settled original
        // group through PipeProcess. A signal result or root exit alone is not
        // enough, and no ProcessOwner errno rule is relaxed.
        let drain_error = self.drain(DrainMode::Cleanup).await.err();
        let input_error = if self.process.exit().is_some() {
            self.payload_inflight = false;
            self.clear_input()
        } else {
            None
        };
        if diagnostic.is_some()
            && error.cleanup_error.is_none()
            && drain_error.is_none()
            && input_error.is_none()
        {
            if error.termination_error.is_none() {
                error.termination_error = diagnostic;
            }
        } else if let Some(cause) = termination_error {
            error.add_cleanup(cause.to_string());
        }
        if let Some(cause) = drain_error {
            error.add_cleanup(cause.to_string());
        }
        if let Some(cause) = input_error {
            error.add_cleanup(cause);
        }
        let detail = self.error(&error.code, error.message.clone());
        error.instance_id = detail.instance_id;
        error.request_id = detail.request_id;
        error.exit_code = detail.exit_code;
        error.signal = detail.signal;
        // A test can cancel the real service future after actual settlement,
        // before its caller observes completion. This never fabricates cleanup.
        #[cfg(all(test, target_os = "macos"))]
        if let Some((entered, release)) = self.cleanup_pause.as_ref() {
            entered.notify_one();
            release.notified().await;
        }
    }

    fn terminate_failed(&mut self) -> Result<(), crate::agent::process::ProcessError> {
        #[cfg(test)]
        if let Some(error) = self.initial_termination_error.take() {
            return Err(error);
        }
        self.process.terminate()
    }

    pub(crate) async fn shutdown(&mut self) -> NativeResult<()> {
        if self.is_failed() {
            let mut error = self.error(
                "plugin_host_connection_failed",
                "原生插件连接已失效。数据未重放。",
            );
            self.finish_failed(&mut error).await;
            return if error.cleanup_error.is_none() && error.termination_error.is_none() {
                Ok(())
            } else {
                Err(error)
            };
        }
        let request = match self.profile {
            NativeProfile::WritingPreview => self.writing_request(RequestBody::Shutdown),
            NativeProfile::WritingProjects => {
                self.project_request(projects_protocol::RequestBody::Shutdown {})
            }
            NativeProfile::NoteTraceDecision => {
                self.literature_request(literature::RequestBody::Shutdown)
            }
        };
        if let Err(mut error) = self.exchange(request, None, Duration::from_secs(2)).await {
            self.finish_failed(&mut error).await;
            return Err(error);
        }
        self.input.close();
        if let Err(mut error) = self.drain(DrainMode::ProtocolTail).await {
            self.finish_failed(&mut error).await;
            return Err(error);
        }
        if self.process.exit().is_some_and(|exit| exit.code != Some(0)) {
            return Err(self.error(
                "plugin_host_exit_failed",
                "原生插件关闭后返回非零退出状态。",
            ));
        }
        Ok(())
    }
}

impl Drop for NativeHost {
    fn drop(&mut self) {
        self.clear_input();
        if let Some(task) = self.input_staging.take() {
            task.abort();
        }
        if let Some(task) = self.output_reading.take() {
            task.abort();
        }
        self.input.close();
        let _ = self.process.terminate();
        if self.process.exit().is_none()
            && let Some(staged) = self.staged.take()
        {
            // Without reap evidence retain the private executable rather than
            // remove an in-use image. The diagnostic never exposes its path.
            let mut staged = staged;
            let _retained = staged.directory.keep();
            if self.payload_inflight
                && let Some(directory) = staged.input_directory.take()
            {
                let _retained_input = directory.keep();
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod c07_tests;
#[cfg(test)]
mod literature_tests;
#[cfg(test)]
mod tests;
