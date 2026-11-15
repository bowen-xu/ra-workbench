//! Cancel external work without dropping an in-flight durable event commit.
use crate::ai::CancellationSignal;
use research_agent_core::{
    model::{
        ModelCapabilities, ModelError, ModelFuture, ModelRequest, ModelStream, ModelStreamEvent,
        ModelTransport, ToolCall,
    },
    runner::{
        RetryWait, RunError, RunFuture, SamplingContext, ToolExecutor, ToolNameResolution,
        ToolOutput, TurnInputSource, TurnRunResult, TurnRunner,
    },
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone, Default)]
pub struct AgentCancellation(CancellationSignal);
impl AgentCancellation {
    pub(crate) fn from_signal(signal: crate::ai::CancellationSignal) -> Self {
        Self(signal)
    }

    pub fn cancel(&self) {
        self.0.cancel();
    }
    pub(crate) async fn cancelled(&self) {
        self.0.cancelled().await;
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
    pub(crate) fn commit_if_active<T>(&self, commit: impl FnOnce() -> T) -> Result<T, ()> {
        self.0.commit_if_active(commit).map_err(|_| ())
    }
}

/// Shared registry for short-lived host operations that have not yet been
/// represented by a conversation task. It keeps the request-id lifecycle in
/// one place while the actual operation still owns the durable completion
/// boundary. `exclusive` matches the existing review/run policy; `false`
/// permits independent request IDs such as reading turns.
#[derive(Clone, Default)]
pub(crate) struct CancellationRegistry {
    active: Arc<Mutex<HashMap<String, crate::ai::CancellationSignal>>>,
}

impl CancellationRegistry {
    pub(crate) fn register(
        &self,
        request_id: &str,
        exclusive: bool,
    ) -> Result<crate::ai::CancellationSignal, ()> {
        if request_id.trim().is_empty() || request_id.len() > 200 {
            return Err(());
        }
        let mut active = self.active.lock().map_err(|_| ())?;
        if active.contains_key(request_id) || exclusive && !active.is_empty() {
            return Err(());
        }
        let signal = crate::ai::CancellationSignal::default();
        active.insert(request_id.to_owned(), signal.clone());
        Ok(signal)
    }

    pub(crate) fn cancel(&self, request_id: &str) -> Result<bool, ()> {
        let active = self.active.lock().map_err(|_| ())?;
        if let Some(signal) = active.get(request_id.trim()) {
            signal.cancel();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn remove(&self, request_id: &str) -> Result<(), ()> {
        self.active.lock().map_err(|_| ())?.remove(request_id);
        Ok(())
    }
}

/// The task owner signals cancellation and awaits this function instead of
/// aborting it. EventSink is deliberately not wrapped: durable writes complete
/// before the runner records Canceled. A completed turn is never relabeled.
pub async fn run_cancellable_turn(
    runner: TurnRunner<'_>,
    turn_id: Uuid,
    request: ModelRequest,
    cancellation: AgentCancellation,
) -> Result<TurnRunResult, RunError> {
    run_cancellable_turn_with_input_and_wait(runner, turn_id, request, cancellation, None, None)
        .await
}

pub async fn run_cancellable_turn_with_input(
    runner: TurnRunner<'_>,
    turn_id: Uuid,
    request: ModelRequest,
    cancellation: AgentCancellation,
    input: Option<&dyn TurnInputSource>,
) -> Result<TurnRunResult, RunError> {
    run_cancellable_turn_with_input_and_wait(runner, turn_id, request, cancellation, input, None)
        .await
}

pub async fn run_cancellable_turn_with_input_and_wait(
    runner: TurnRunner<'_>,
    turn_id: Uuid,
    request: ModelRequest,
    cancellation: AgentCancellation,
    input: Option<&dyn TurnInputSource>,
    retry_wait: Option<&dyn RetryWait>,
) -> Result<TurnRunResult, RunError> {
    run_cancellable_turn_with_sampling_context(
        runner,
        turn_id,
        request,
        cancellation,
        input,
        retry_wait,
        None,
    )
    .await
}

pub(crate) async fn run_cancellable_turn_with_sampling_context(
    runner: TurnRunner<'_>,
    turn_id: Uuid,
    request: ModelRequest,
    cancellation: AgentCancellation,
    input: Option<&dyn TurnInputSource>,
    retry_wait: Option<&dyn RetryWait>,
    context: Option<&dyn SamplingContext>,
) -> Result<TurnRunResult, RunError> {
    let transport = CancellableTransport {
        inner: runner.transport,
        cancellation: cancellation.clone(),
    };
    let tools = CancellableTools {
        inner: runner.tools,
        cancellation: cancellation.clone(),
    };
    let runner = TurnRunner {
        transport: &transport,
        tools: &tools,
        sink: runner.sink,
        ids: runner.ids,
        limits: runner.limits,
    };
    let input = CancellableInput {
        inner: input,
        cancellation,
    };
    match retry_wait {
        Some(wait) => {
            runner
                .run_with_sampling_context(turn_id, request, &input, wait, context)
                .await
        }
        None => {
            runner
                .run_with_sampling_context(turn_id, request, &input, &ImmediateRetryWait, context)
                .await
        }
    }
}

struct ImmediateRetryWait;
impl RetryWait for ImmediateRetryWait {
    fn wait(&self, _: u64) -> RunFuture<'_, ()> {
        Box::pin(async { Ok(()) })
    }
}

struct CancellableInput<'a> {
    inner: Option<&'a dyn TurnInputSource>,
    cancellation: AgentCancellation,
}
impl TurnInputSource for CancellableInput<'_> {
    fn seal_for_stop(&self, turn_id: Uuid) -> RunFuture<'_, ()> {
        Box::pin(async move {
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            if let Some(input) = self.inner {
                input.seal_for_stop(turn_id).await?;
            }
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            Ok(())
        })
    }
    fn has_pending(&self, turn_id: Uuid) -> RunFuture<'_, bool> {
        Box::pin(async move {
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            let pending = match self.inner {
                Some(input) => input.has_pending(turn_id).await?,
                None => false,
            };
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            Ok(pending)
        })
    }
    fn has_pending_or_seal(&self, turn_id: Uuid) -> RunFuture<'_, bool> {
        Box::pin(async move {
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            let pending = match self.inner {
                Some(input) => input.has_pending_or_seal(turn_id).await?,
                None => false,
            };
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            Ok(pending)
        })
    }
    fn drain(
        &self,
        turn_id: Uuid,
        finish_if_empty: bool,
    ) -> RunFuture<'_, Vec<research_agent_core::model::ModelMessage>> {
        Box::pin(async move {
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            let messages = match self.inner {
                Some(input) => input.drain(turn_id, finish_if_empty).await?,
                None => Vec::new(),
            };
            // Never race/drop the durable input commit itself.
            if self.cancellation.is_cancelled() {
                return Err(ModelError::Canceled.into());
            }
            Ok(messages)
        })
    }
}

struct CancellableTransport<'a> {
    inner: &'a dyn ModelTransport,
    cancellation: AgentCancellation,
}
impl ModelTransport for CancellableTransport<'_> {
    fn tool_output_history_prepared(&self) -> bool {
        self.inner.tool_output_history_prepared()
    }
    fn tool_output_policy(
        &self,
    ) -> Option<research_agent_core::model::tool_output::ToolOutputPolicy> {
        self.inner.tool_output_policy()
    }
    fn pre_turn_compaction<'a>(
        &'a self,
        turn: uuid::Uuid,
        model: &'a research_agent_core::protocol::ModelIdentity,
        active: Option<u64>,
    ) -> ModelFuture<
        'a,
        Option<Arc<dyn research_agent_core::context::compaction_strategy::CompactionStrategy>>,
    > {
        Box::pin(async move {
            self.cancellation
                .0
                .race(self.inner.pre_turn_compaction(turn, model, active))
                .await
                .map_err(|_| ModelError::Canceled)?
        })
    }

    fn prepare_context_policy(&self) -> ModelFuture<'_, ()> {
        Box::pin(async move {
            self.cancellation
                .0
                .race(self.inner.prepare_context_policy())
                .await
                .map_err(|_| ModelError::Canceled)?
        })
    }
    fn context_accounting(
        &self,
    ) -> Option<Arc<research_agent_core::context::accounting::ContextAccounting>> {
        self.inner.context_accounting()
    }
    fn capabilities(&self) -> ModelCapabilities {
        self.inner.capabilities()
    }
    fn supports_namespace_tools(&self) -> bool {
        self.inner.supports_namespace_tools()
    }
    fn start_text_compaction(
        &self,
        request: ModelRequest,
    ) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            let inner = self
                .cancellation
                .0
                .race(self.inner.start_text_compaction(request))
                .await
                .map_err(|_| ModelError::Canceled)??;
            Ok(Box::new(CancellableStream {
                inner,
                cancellation: self.cancellation.clone(),
            }) as Box<dyn ModelStream>)
        })
    }
    fn start(&self, request: ModelRequest) -> ModelFuture<'_, Box<dyn ModelStream>> {
        Box::pin(async move {
            let inner = self
                .cancellation
                .0
                .race(self.inner.start(request))
                .await
                .map_err(|_| ModelError::Canceled)??;
            Ok(Box::new(CancellableStream {
                inner,
                cancellation: self.cancellation.clone(),
            }) as Box<dyn ModelStream>)
        })
    }
}
struct CancellableStream {
    inner: Box<dyn ModelStream>,
    cancellation: AgentCancellation,
}
impl ModelStream for CancellableStream {
    fn next(&mut self) -> ModelFuture<'_, Option<ModelStreamEvent>> {
        Box::pin(async move {
            self.cancellation
                .0
                .race(self.inner.next())
                .await
                .map_err(|_| ModelError::Canceled)?
        })
    }
}
struct CancellableTools<'a> {
    inner: &'a dyn ToolExecutor,
    cancellation: AgentCancellation,
}
impl ToolExecutor for CancellableTools<'_> {
    fn child_creation_intent(
        &self,
        call: &ToolCall,
    ) -> Option<research_agent_core::conversation::CreateBoundConversation> {
        self.inner.child_creation_intent(call)
    }
    fn resolve_name(&self, name: &str) -> ToolNameResolution {
        self.inner.resolve_name(name)
    }
    fn resolve_qualified_name(&self, namespace: Option<&str>, name: &str) -> ToolNameResolution {
        self.inner.resolve_qualified_name(namespace, name)
    }
    fn supports_parallel(&self, call: &ToolCall) -> bool {
        self.inner.supports_parallel(call)
    }
    fn cancellation_safe_to_drop(&self, call: &ToolCall) -> bool {
        self.inner.cancellation_safe_to_drop(call)
    }
    fn requires_result_receipt(&self, call: &ToolCall) -> bool {
        self.inner.requires_result_receipt(call)
    }
    fn cancel_pending(&self, call: ToolCall) -> RunFuture<'_, ()> {
        self.inner.cancel_pending(call)
    }
    fn finalization_call(&self) -> RunFuture<'_, Option<ToolCall>> {
        Box::pin(async {
            if self.cancellation.is_cancelled() {
                return Err(RunError::Model(ModelError::Canceled));
            }
            // The inner inspection may own a blocking worker or lease. Await it
            // fully before honoring a cancellation that arrived in flight.
            let call = self.inner.finalization_call().await?;
            if self.cancellation.is_cancelled() {
                return Err(RunError::Model(ModelError::Canceled));
            }
            Ok(call)
        })
    }
    fn release_for_finalization(&self) -> RunFuture<'_, bool> {
        // The owner may already be handing off a lease. Cancellation cannot
        // discard the acknowledgement of that handoff.
        self.inner.release_for_finalization()
    }
    fn result_committed(&self, call: ToolCall) -> RunFuture<'_, ()> {
        // This follows durable acknowledgement; cancellation must not drop
        // release bookkeeping or turn a completed tool into a replay request.
        self.inner.result_committed(call)
    }
    fn validate(&self, call: &ToolCall) -> Result<(), RunError> {
        if self.cancellation.is_cancelled() {
            return Err(ModelError::Canceled.into());
        }
        self.inner.validate(call)
    }
    fn authorize_and_execute(&self, call: ToolCall) -> RunFuture<'_, ToolOutput> {
        Box::pin(async move {
            if self.cancellation.is_cancelled() {
                return Err(RunError::Model(ModelError::Canceled));
            }
            let safe_to_drop = self.inner.cancellation_safe_to_drop(&call);
            let execution = self.inner.authorize_and_execute(call.clone());
            tokio::pin!(execution);
            // Codex tools/parallel.rs checks a completed dispatch before an
            // abort. Keep the real result when both completion and cancel are
            // ready; it still needs the normal durable result/receipt path.
            tokio::select! {
                biased;
                result = &mut execution => result,
                _ = self.cancellation.cancelled() => {
                    if safe_to_drop {
                        Err(RunError::Model(ModelError::Canceled))
                    } else {
                        // The task owner has signaled process cancellation and
                        // closed admission. Keep polling its teardown, blocking
                        // worker or durable editor/delivery acknowledgement.
                        let notified = self.inner.cancel_pending(call).await;
                        let result = execution.await;
                        match (notified, result) {
                            (_, Ok(output)) => Ok(output),
                            (Err(error), Err(_)) | (Ok(()), Err(error)) => Err(error),
                        }
                    }
                }
            }
        })
    }
}

#[cfg(all(test, unix))]
#[path = "cancellation/mcp_tests.rs"]
mod mcp_tests;
#[cfg(all(test, unix))]
#[path = "cancellation/shell_tests.rs"]
mod shell_tests;
#[cfg(test)]
#[path = "cancellation/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "cancellation/tools_tests.rs"]
mod tools_tests;
