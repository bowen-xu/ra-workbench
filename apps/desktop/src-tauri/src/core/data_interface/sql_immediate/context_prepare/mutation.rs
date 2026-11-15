//! Original preparation transaction with request-local Native source pauses.
use super::*;
use crate::{
    agent::{
        context_catalog::ContextCatalogFailure,
        writing_context::{ContextNeed, WritingContextError, WritingContextReader},
    },
    ai::structured_resource_validation::Flow,
};

/// The original SQL shell settled before returning this consumed scope failure.
/// Keep both the SQL outcome and exact catalog diagnostic, including unknown cleanup.
pub(crate) struct ContextPrepareScopeFailure {
    pub(crate) error: SqlImmediateFailure,
    pub(crate) catalog: Box<ContextCatalogFailure>,
}
type Prepared = (context_session::Preview, SqlImmediateReceipt);
type Attempt =
    Result<Result<Prepared, ContextPrepareScopeFailure>, Flow<SqlImmediateFailure, ContextNeed>>;

impl Database {
    pub(crate) fn prepare_context_owned(
        &self,
        command: &ContextPrepareIntent,
        source: ContextPrepareSource,
    ) -> Result<Prepared, SqlImmediateFailure> {
        self.prepare_context_with_fault(
            command,
            source,
            #[cfg(test)]
            super::super::Fault::None,
        )
    }
    pub(super) fn prepare_context_with_fault(
        &self,
        command: &ContextPrepareIntent,
        source: ContextPrepareSource,
        #[cfg(test)] fault: super::super::Fault,
    ) -> Result<Prepared, SqlImmediateFailure> {
        self.prepare_context_attempt(
            command,
            source,
            None,
            #[cfg(test)]
            fault,
        )
        .map_err(|flow| match flow {
            Flow::Fail(failure) => failure.error,
            Flow::Need(_) => unreachable!("static ordinary preparation cannot need Native"),
        })?
        .map_err(|failure| failure.error)
    }
    pub(crate) fn prepare_context_with_reader(
        &self,
        command: &ContextPrepareIntent,
        source: ContextPrepareSource,
        writing: &mut dyn WritingContextReader,
    ) -> Attempt {
        self.prepare_context_attempt(
            command,
            source,
            Some(writing),
            #[cfg(test)]
            super::super::Fault::None,
        )
    }
    fn prepare_context_attempt(
        &self,
        command: &ContextPrepareIntent,
        source: ContextPrepareSource,
        writing: Option<&mut dyn WritingContextReader>,
        #[cfg(test)] fault: super::super::Fault,
    ) -> Attempt {
        if !std::ptr::eq(
            self,
            std::sync::Arc::as_ptr(&source.store.shared_agent_connection()),
        ) {
            return Err(Flow::original(SqlImmediateFailure::Domain(
                CatalogError::InvalidInput,
            )));
        }
        let identity = identity(command).map_err(Flow::original)?;
        let mut preview = None;
        let mut scope_failure = None;
        let mut pause = writing_source::SourcePause::new(
            command.request.request_id,
            "context-prepare-source-pause",
        );
        let executed = self.execute_sql(
            command.request.request_id,
            |tx, fence| {
                check_epoch(command, fence)?;
                receipts::check_state(tx, command.request.request_id, fence, "context-prepare")?;
                receipts::check_cross_kind(
                    tx,
                    command.request.request_id,
                    fence,
                    "context-prepare",
                )?;
                let prior =
                    receipts::lookup_with_result(tx, &identity, fence, "context-prepare", false)?;
                if let Some(value) = replay_in(tx, command, fence, prior.as_ref())? {
                    preview = Some(value);
                    return Ok(prior.expect("receipt required").0);
                }
                let prepared = context_session::materialize_with_context(
                    tx,
                    &source.store,
                    command.request.clone(),
                    source.model,
                    writing,
                )
                .map_err(|cause| match cause {
                    context_session::sources::MaterializationError::Source(
                        context_session::Error::NativeCatalogUnavailable(catalog),
                    ) => {
                        let cause = error(
                            command,
                            fence,
                            context_session::Error::NativeCatalogUnavailable(catalog.clone()),
                        );
                        scope_failure = Some(catalog);
                        Flow::original(cause)
                    }
                    context_session::sources::MaterializationError::Source(cause) => {
                        Flow::original(error(command, fence, cause))
                    }
                    context_session::sources::MaterializationError::Writing(flow) => flow
                        .map_error(|cause| {
                            error(
                                command,
                                fence,
                                match cause {
                                    WritingContextError::InvalidInput => {
                                        context_session::Error::InvalidInput
                                    }
                                    WritingContextError::Conflict => {
                                        context_session::Error::ConfirmationMismatch
                                    }
                                    WritingContextError::Unavailable => {
                                        context_session::Error::SourceUnavailable
                                    }
                                    WritingContextError::Budget => context_session::Error::Budget,
                                },
                            )
                        }),
                });
                preview = Some(pause.capture(prepared, fence)?);
                let result_hash = preview_hash(preview.as_ref().expect("prepared result"))
                    .map_err(|_| {
                        SqlImmediateFailure::Invalid(Box::new(diagnostic(
                            command.request.request_id,
                            fence,
                            "context-prepare",
                            "context_prepare_result_encoding_invalid",
                        )))
                    })?;
                let receipt = receipts::insert(tx, &identity, fence)?;
                let updated = tx.execute(
                    "update data_sql_operation_receipts set result_json=?2 where operation_id=?1",
                    params![
                        command.request.request_id.to_string(),
                        serde_json::json!({"sha256":result_hash}).to_string()
                    ],
                )
                .map_err(|cause| {
                    receipts::storage(
                        command.request.request_id,
                        fence,
                        "context-prepare-result",
                        &cause,
                    )
                })?;
                if updated != 1 {
                    return Err(SqlImmediateFailure::Corrupt(Box::new(diagnostic(
                        command.request.request_id,
                        fence,
                        "context-prepare-result",
                        "context_prepare_result_proof_not_written",
                    ))));
                }
                Ok(receipt)
            },
            #[cfg(test)]
            fault,
        );
        let result = pause.finish(executed.map(|receipt| {
            (
                preview.expect("successful preparation has a result"),
                receipt,
            )
        }));
        match (result, scope_failure) {
            (Err(Flow::Fail(failure)), Some(catalog)) => Ok(Err(ContextPrepareScopeFailure {
                error: failure.error,
                catalog,
            })),
            (result, _) => result.map(Ok),
        }
    }
}

#[cfg(test)]
#[path = "mutation/tests.rs"]
mod tests;
