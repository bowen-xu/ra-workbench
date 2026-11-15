//! Cancellation-safe input preparation using the same finite request owner.
use super::*;

impl NativeHost {
    pub(super) async fn stage_input(
        &mut self,
        wire: &mut WireRequest,
        input: NativeInput,
    ) -> NativeResult<()> {
        if !matches!(
            (&*wire, &input),
            (
                WireRequest::Literature(_),
                NativeInput::Trace(_) | NativeInput::Catalog(_)
            ) | (WireRequest::Projects(_), NativeInput::Projects(_))
        ) {
            return Err(self.error(
                "plugin_host_profile_unsupported",
                "输入不属于当前原生协议。",
            ));
        }
        let directory = self
            .staged
            .as_ref()
            .and_then(|stage| stage.input_directory.as_ref())
            .map(|directory| directory.path().to_owned())
            .ok_or_else(|| {
                self.error("plugin_host_snapshot_stage_failed", "原生输入目录不可用。")
            })?;
        let identity = wire.clone();
        let cleanup = Arc::clone(&self.input_cleanup);
        let catalog_operation = match &input {
            NativeInput::Catalog(input) => Some(input.operation()),
            _ => None,
        };
        let project_operation = match &input {
            NativeInput::Projects(input) => Some(input.operation()),
            _ => None,
        };
        self.payload_inflight = catalog_operation.is_some() || project_operation.is_some();
        let profile = self.profile;
        self.input_staging = Some(tokio::task::spawn_blocking(move || {
            match (identity, input) {
                (WireRequest::Literature(request), NativeInput::Trace(input)) => {
                    InputLease::stage(&directory, &request, &input, cleanup)
                }
                (WireRequest::Literature(request), NativeInput::Catalog(input)) => {
                    InputLease::stage_catalog(&directory, &request, &input, cleanup)
                }
                (WireRequest::Projects(request), NativeInput::Projects(input)) => {
                    InputLease::stage_projects(&directory, &request, &input, cleanup)
                }
                _ => Err(NativeHostError::for_profile(
                    profile,
                    "plugin_host_profile_unsupported",
                    "输入不属于当前原生协议。",
                )),
            }
        }));
        // Keep the JoinHandle in the host during this await. Dropping the
        // request cannot detach an unknown input writer from cleanup evidence.
        let result = match self.input_staging.as_mut() {
            Some(task) => task.await,
            None => {
                return Err(self.error(
                    "plugin_host_snapshot_stage_failed",
                    "原生输入准备任务不可用。",
                ));
            }
        };
        self.input_staging.take();
        let (lease, reference) = result.map_err(|_| {
            self.error(
                "plugin_host_snapshot_stage_failed",
                "原生输入准备任务未完成。",
            )
        })??;
        self.input_lease = Some(lease);
        match wire {
            WireRequest::Literature(request) => {
                request.body = match catalog_operation {
                    Some(operation) => literature::RequestBody::Catalog {
                        operation,
                        snapshot: reference,
                    },
                    None => literature::RequestBody::DecideNoteTrace {
                        snapshot: reference,
                    },
                }
            }
            WireRequest::Projects(request) => {
                request.body = projects_protocol::RequestBody::Projects {
                    operation: project_operation.ok_or_else(|| {
                        self.error("plugin_host_profile_unsupported", "写作输入阶段不匹配。")
                    })?,
                    snapshot: reference,
                }
            }
            _ => {
                return Err(self.error(
                    "plugin_host_profile_unsupported",
                    "当前协议不接受快照输入。",
                ));
            }
        }
        Ok(())
    }

    pub(super) fn clear_input(&mut self) -> Option<String> {
        // A child may not have opened its result file yet. Preserve its exact
        // cleanup name until a valid reply or confirmed process exit, so a
        // cancellation cannot miss a file created after an early unlink.
        if self.payload_inflight {
            return self.input_cleanup.failure();
        }
        if let Some(lease) = self.input_lease.as_mut()
            && lease.clear().is_ok()
        {
            self.input_lease = None;
        }
        self.input_cleanup.failure()
    }

    pub(super) async fn finish_input(&mut self) {
        self.finish_payload_read().await;
        let outcome = match self.input_staging.as_mut() {
            Some(task) => timeout(DRAIN_WAIT, task).await,
            None => return,
        };
        match outcome {
            Ok(Ok(result)) => {
                self.input_staging.take();
                if let Ok((mut lease, _)) = result {
                    let _ = lease.clear();
                }
            }
            Ok(Err(_)) => {
                self.input_staging.take();
                self.input_cleanup.record(std::io::ErrorKind::Other);
            }
            Err(_) => {
                if let Some(task) = self.input_staging.take() {
                    task.abort();
                }
                // A running blocking task cannot be aborted reliably. Its
                // eventual lease still drops, but this instance cannot claim
                // cleanup or allow a replacement before that is confirmed.
                self.input_cleanup.record(std::io::ErrorKind::TimedOut);
            }
        }
    }
}
