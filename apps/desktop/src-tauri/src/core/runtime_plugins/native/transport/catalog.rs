//! The same request owner reads a verified result file before releasing its lease.
use super::*;
use research_workbench_native_sdk::catalog::CatalogOperation;

impl NativeHost {
    pub(crate) async fn catalog_call(
        &mut self,
        input: CatalogInput,
        wait: Duration,
    ) -> NativeResult<CatalogOutput> {
        if self.profile != NativeProfile::NoteTraceDecision {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "当前原生实例不接受文献目录协议。",
            ));
        }
        let request = self.literature_request(literature::RequestBody::Initialize);
        match self
            .exchange(request, Some(NativeInput::Catalog(input)), wait)
            .await?
        {
            WireResponse::Catalog(output) => Ok(output),
            _ => Err(self.error("plugin_host_protocol_invalid", "原生插件未返回目录结果。")),
        }
    }

    pub(super) async fn read_catalog_response(
        &mut self,
        response: WireResponse,
        operation: CatalogOperation,
    ) -> NativeResult<WireResponse> {
        let WireResponse::Literature(literature::ResponseBody::CatalogResult { payload }) =
            response
        else {
            return Err(self.error("plugin_host_protocol_invalid", "原生目录结果种类不匹配。"));
        };
        let directory = self
            .staged
            .as_ref()
            .and_then(|stage| stage.input_directory.as_ref())
            .map(|directory| directory.path().to_owned())
            .ok_or_else(|| self.error("plugin_host_payload_invalid", "目录结果拥有者不可用。"))?;
        self.output_reading = Some(tokio::task::spawn_blocking(move || {
            read_result(&directory, &payload).map(WireResponse::Catalog)
        }));
        let output = match self.output_reading.as_mut() {
            Some(task) => task.await,
            None => {
                return Err(self.error("plugin_host_payload_invalid", "目录结果读取任务不可用。"));
            }
        };
        self.output_reading.take();
        let output = output
            .map_err(|_| self.error("plugin_host_payload_invalid", "目录结果读取任务未完成。"))??;
        let WireResponse::Catalog(output) = output else {
            return Err(self.error(
                "plugin_host_response_mismatch",
                "目录读取任务返回其他协议结果。",
            ));
        };
        if output.operation() != operation {
            return Err(self.error(
                "plugin_host_response_mismatch",
                "目录结果与本次阶段不匹配。",
            ));
        }
        self.payload_inflight = false;
        Ok(WireResponse::Catalog(output))
    }

    pub(super) async fn finish_payload_read(&mut self) {
        let outcome = match self.output_reading.as_mut() {
            Some(task) => timeout(DRAIN_WAIT, task).await,
            None => return,
        };
        match outcome {
            Ok(Ok(_)) => {
                self.output_reading.take();
            }
            Ok(Err(_)) => {
                self.output_reading.take();
                self.input_cleanup.record(std::io::ErrorKind::Other);
            }
            Err(_) => {
                if let Some(task) = self.output_reading.take() {
                    task.abort();
                }
                self.input_cleanup.record(std::io::ErrorKind::TimedOut);
            }
        }
    }
}

/// Hash exactly the bytes parsed, with no frame-derived limit or path disclosure.
#[cfg(unix)]
fn read_result(
    directory: &std::path::Path,
    reference: &literature::SnapshotRef,
) -> NativeResult<CatalogOutput> {
    read_payload(
        directory,
        reference,
        NativeProfile::NoteTraceDecision,
        "目录结果",
    )
}

#[cfg(unix)]
pub(super) fn read_payload<T: serde::de::DeserializeOwned>(
    directory: &std::path::Path,
    reference: &literature::SnapshotRef,
    profile: NativeProfile,
    label: &str,
) -> NativeResult<T> {
    use research_workbench_native_sdk::streaming_payload::{self, PayloadError};
    use std::{fs, io, os::unix::fs::OpenOptionsExt};
    let invalid = || {
        NativeHostError::for_profile(
            profile,
            "plugin_host_payload_invalid",
            format!("{label}的内容、长度或摘要无效。"),
        )
        .package(&reference.package_key)
    };
    let io_failure = |error: io::Error| {
        let mut diagnostic = invalid();
        diagnostic.message = format!("{label}读取失败：{:?}", error.kind());
        diagnostic
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(directory.join(&reference.relative_handle))
        .map_err(io_failure)?;
    let metadata = file.metadata().map_err(io_failure)?;
    if !metadata.is_file() || metadata.len() != reference.byte_size {
        return Err(invalid());
    }
    streaming_payload::read_json(file, reference.byte_size, &reference.sha256).map_err(|error| {
        let mut diagnostic = invalid();
        diagnostic.message = match error {
            PayloadError::Invalid {
                io_kind: Some(kind),
                ..
            }
            | PayloadError::Io(kind) => format!("{label}读取失败：{kind:?}"),
            PayloadError::Invalid {
                category,
                line,
                column,
                ..
            } => format!("{label}解析失败：{category}（行{line}，列{column}）。"),
            _ => return diagnostic,
        };
        diagnostic
    })
}

#[cfg(test)]
mod tests;
