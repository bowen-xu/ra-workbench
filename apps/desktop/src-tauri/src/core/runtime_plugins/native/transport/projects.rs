//! Writing uses the same transport, request lease and verified result reader.
use super::*;

impl NativeHost {
    pub(crate) async fn project_call(
        &mut self,
        input: writing_project::Input,
        wait: Duration,
    ) -> NativeResult<writing_project::Output> {
        if self.profile != NativeProfile::WritingProjects {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "当前原生实例不接受写作工程协议。",
            ));
        }
        let request = self.project_request(projects_protocol::RequestBody::Initialize {});
        match self
            .exchange(request, Some(NativeInput::Projects(input)), wait)
            .await?
        {
            WireResponse::Projects(output) => Ok(output),
            _ => Err(self.error(
                "plugin_host_protocol_invalid",
                "原生插件未返回写作工程结果。",
            )),
        }
    }

    pub(crate) async fn project_preview_call(
        &mut self,
        source_path: String,
        wait: Duration,
    ) -> NativeResult<protocol::ImportPreview> {
        if self.profile != NativeProfile::WritingProjects {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "当前原生实例不接受写作工程协议。",
            ));
        }
        let request =
            self.project_request(projects_protocol::RequestBody::PreviewImport { source_path });
        match self.exchange(request, None, wait).await? {
            WireResponse::ProjectBody(projects_protocol::ResponseBody::PreviewImportResult {
                outcome: protocol::PreviewOutcome::Success { preview },
            }) => Ok(preview),
            WireResponse::ProjectBody(projects_protocol::ResponseBody::PreviewImportResult {
                outcome: protocol::PreviewOutcome::Failure { error },
            }) => Err(self.domain_error(error)),
            _ => Err(self.error(
                "plugin_host_protocol_invalid",
                "原生插件未返回导入预览结果。",
            )),
        }
    }

    pub(super) async fn read_project_response(
        &mut self,
        response: WireResponse,
        expected: writing_project::Input,
    ) -> NativeResult<WireResponse> {
        let payload = match response {
            WireResponse::ProjectBody(projects_protocol::ResponseBody::ProjectResult {
                payload,
            }) => payload,
            WireResponse::ProjectBody(projects_protocol::ResponseBody::ProjectFailure {
                failure,
                message,
            }) => {
                self.payload_inflight = false;
                return Err(self.error(failure.code(), message));
            }
            _ => return Err(self.error("plugin_host_protocol_invalid", "原生写作结果种类不匹配。")),
        };
        let directory = self
            .staged
            .as_ref()
            .and_then(|stage| stage.input_directory.as_ref())
            .map(|directory| directory.path().to_owned())
            .ok_or_else(|| self.error("plugin_host_payload_invalid", "写作结果拥有者不可用。"))?;
        self.output_reading = Some(tokio::task::spawn_blocking(move || {
            catalog::read_payload(
                &directory,
                &payload,
                NativeProfile::WritingProjects,
                "写作工程结果",
            )
            .map(WireResponse::Projects)
        }));
        let output = match self.output_reading.as_mut() {
            Some(task) => task.await,
            None => {
                return Err(self.error("plugin_host_payload_invalid", "写作结果读取任务不可用。"));
            }
        };
        self.output_reading.take();
        let output = output
            .map_err(|_| self.error("plugin_host_payload_invalid", "写作结果读取任务未完成。"))??;
        let WireResponse::Projects(output) = output else {
            return Err(self.error(
                "plugin_host_response_mismatch",
                "写作读取任务返回其他协议结果。",
            ));
        };
        if output.input != expected {
            return Err(self.error(
                "plugin_host_response_mismatch",
                "写作结果与本次完整输入不匹配。",
            ));
        }
        self.payload_inflight = false;
        Ok(WireResponse::Projects(output))
    }
}
