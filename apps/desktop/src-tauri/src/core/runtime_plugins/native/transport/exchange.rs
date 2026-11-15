//! One bounded pipe exchange and its cancellation guard.
use super::*;

struct RequestGuard<'a> {
    host: &'a mut NativeHost,
    completed: bool,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.host.clear_input();
            self.host.poison();
        }
    }
}

impl NativeHost {
    pub(super) async fn exchange(
        &mut self,
        mut request: WireRequest,
        snapshot: Option<NativeInput>,
        wait: Duration,
    ) -> NativeResult<WireResponse> {
        if self.is_failed() {
            let mut error = self.error(
                "plugin_host_connection_failed",
                "原生插件连接已失效，旧请求不会重放。",
            );
            if let Some(cause) = self.input_cleanup.failure() {
                error.add_cleanup(cause);
            }
            return Err(error);
        }
        self.last_request_id = Some(
            match &request {
                WireRequest::Writing(r) => &r.request_id,
                WireRequest::Literature(r) => &r.request_id,
                WireRequest::Projects(r) => &r.request_id,
            }
            .clone(),
        );
        let catalog_operation = match &snapshot {
            Some(NativeInput::Catalog(input)) => Some(input.operation()),
            _ => None,
        };
        let project_input = match &snapshot {
            Some(NativeInput::Projects(input)) => Some(input.clone()),
            _ => None,
        };
        let mut guard = RequestGuard {
            host: self,
            completed: false,
        };
        let mut result = match timeout(wait, async {
            if let Some(snapshot) = snapshot {
                guard.host.stage_input(&mut request, snapshot).await?;
            }
            let response = guard.host.request_inner(&request).await?;
            if let Some(operation) = catalog_operation {
                guard.host.read_catalog_response(response, operation).await
            } else if let Some(expected) = project_input {
                guard.host.read_project_response(response, expected).await
            } else {
                Ok(response)
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(guard.host.error(
                "plugin_host_timeout",
                "原生插件未在期限内回复；此请求不会自动重放。",
            )),
        };
        if let Some(cause) = guard.host.clear_input() {
            match &mut result {
                Err(error) => error.add_cleanup(cause),
                Ok(_) => {
                    let mut error = guard.host.error(
                        "plugin_host_input_cleanup_failed",
                        "原生输入未确认清理；本次不能报告完成。",
                    );
                    error.add_cleanup(cause);
                    result = Err(error);
                }
            }
        }
        if let Err(error) = &mut result {
            let identity = guard.host.error(&error.code, error.message.clone());
            error.instance_id = identity.instance_id;
            error.request_id = identity.request_id;
            guard.host.poison();
        }
        guard.completed = true;
        result
    }

    async fn request_inner(&mut self, request: &WireRequest) -> NativeResult<WireResponse> {
        let bytes = request
            .encode()
            .map_err(|error| self.error(error.code, error.message))?;
        let mut written = 0;
        while written < bytes.len() {
            let count = self
                .input
                .write(&bytes[written..])
                .await
                .map_err(|error| self.error("plugin_host_input_failed", error.to_string()))?;
            if count == 0 {
                return Err(
                    self.error("plugin_host_input_closed", "原生插件在接收请求前关闭输入。")
                );
            }
            written += count;
        }
        loop {
            let chunk = self
                .process
                .next_output()
                .await
                .map_err(|error| self.error("plugin_host_output_failed", error.to_string()))?
                .ok_or_else(|| self.error("plugin_host_exited", "原生插件在回复前退出。"))?;
            if chunk.stream == OutputStream::Stderr {
                self.stderr_bytes = self.stderr_bytes.saturating_add(chunk.bytes.len());
                if self.stderr_bytes > STDERR_BUDGET {
                    return Err(self.error(
                        "plugin_host_output_limit",
                        "原生插件超出诊断输出预算；原始输出未保存。",
                    ));
                }
                continue;
            }
            let frames = self.decoder.push(&chunk.bytes).map_err(|_| {
                self.error(
                    "plugin_host_output_limit",
                    "原生插件输出为空帧或超出64MiB帧预算。",
                )
            })?;
            if frames.is_empty() {
                continue;
            }
            if frames.len() != 1 || !self.decoder.is_empty() {
                return Err(self.error(
                    "plugin_host_protocol_invalid",
                    "原生插件发送了未关联的额外回复。",
                ));
            }
            return request
                .decode_response(&frames[0])
                .map_err(|error| self.error(error.code, error.message));
        }
    }
}
