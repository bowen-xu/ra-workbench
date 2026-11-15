use super::*;
use crate::agent::writing_context::ContextNeed;
use crate::agent::writing_context::WritingContextError as C;
use crate::{
    agent::writing_context::WritingContextReader, ai::structured_resource_validation::Flow,
};
use research_agent_core::submission::SubmissionError as S;
use writing_submission::WritingSubmissionError as W;

impl From<SqlImmediateFailure> for Flow<SqlImmediateFailure, ContextNeed> {
    fn from(error: SqlImmediateFailure) -> Self {
        Self::original(error)
    }
}

pub(super) fn materialization_error(
    command: &WritingPrepareIntent,
    fence: ReadFence,
    cause: W,
) -> SqlImmediateFailure {
    let (class, reason) = match cause {
        W::Storage(cause) => return cause.into_inner(),
        W::Sql(cause) => {
            return receipts::storage(
                command.operation(),
                fence,
                "writing-prepare-materialize",
                &cause,
            );
        }
        W::Unbound => ("invalid", "writing_binding_unavailable"),
        W::Context(C::Conflict) => ("conflict", "writing_context_changed"),
        W::Context(C::InvalidInput) => ("invalid", "writing_context_invalid"),
        W::Context(C::Budget) => ("invalid", "writing_context_budget_exceeded"),
        W::Context(C::Unavailable) => ("unavailable", "writing_context_source_unavailable"),
        W::Input(S::Conflict) => ("conflict", "writing_input_changed"),
        W::Input(S::InvalidInput) => ("invalid", "writing_input_invalid"),
        W::Input(S::Corrupt) => ("corrupt", "writing_input_corrupt"),
        W::Instructions(crate::agent::instructions::InstructionReadError::Unauthorized) => {
            ("conflict", "writing_instructions_unauthorized")
        }
        W::Instructions(crate::agent::instructions::InstructionReadError::InvalidScope) => {
            ("invalid", "writing_instructions_scope_invalid")
        }
        W::Instructions(crate::agent::instructions::InstructionReadError::Budget) => {
            ("invalid", "writing_instructions_budget_exceeded")
        }
        W::Instructions(crate::agent::instructions::InstructionReadError::Encoding) => {
            ("invalid", "writing_instructions_encoding_invalid")
        }
        W::Instructions(_) => ("unavailable", "writing_instructions_unavailable"),
        _ => ("unavailable", "writing_preparation_unavailable"),
    };
    let detail = Box::new(diagnostic(
        command.operation(),
        fence,
        "writing-prepare-materialize",
        reason,
    ));
    match class {
        "conflict" => SqlImmediateFailure::Conflict(detail),
        "invalid" => SqlImmediateFailure::Invalid(detail),
        "corrupt" => SqlImmediateFailure::Corrupt(detail),
        _ => SqlImmediateFailure::Unavailable(detail),
    }
}
pub(super) fn prepare_in(
    tx: &Transaction<'_>,
    command: &WritingPrepareIntent,
    source: WritingPrepareSource,
    fence: ReadFence,
    writing: Option<&mut dyn WritingContextReader>,
) -> Result<SubmissionRecord, Flow<SqlImmediateFailure, ContextNeed>> {
    let wire = source.request.wire_request();
    if serde_json::to_value(&wire)
        .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?
        != serde_json::to_value(&command.request)
            .map_err(|_| SqlImmediateFailure::Domain(CatalogError::InvalidInput))?
    {
        return Err(SqlImmediateFailure::Domain(CatalogError::InvalidInput).into());
    }
    authorize(tx, command, fence)?;
    writing_submission::check_compaction_prepare_in(
        tx,
        command.conversation(),
        command.operation(),
        &command.request.text,
        command.compact_revision,
    )
    .map_err(|cause| materialization_error(command, fence, cause))?;
    let input = writing_submission::materialize_submission_with_context(
        tx,
        &source.store,
        source.request,
        source.model,
        writing,
    )
    .map_err(|flow| flow.map_error(|cause| materialization_error(command, fence, cause)))?;
    use crate::agent::repository::SqlRepositoryError as E;
    let record = submissions::insert_in_sql(tx, command.conversation(), command.operation(), input)
        .map_err(|cause| match cause {
            E::Domain(cause) => materialization_error(command, fence, W::Input(cause)),
            E::Sql(cause) => {
                receipts::storage(command.operation(), fence, "writing-prepare-record", &cause)
            }
        })?;
    // This is a new candidate, so an unprovable scope is rejected input rather
    // than evidence that an already committed store must be blocked.
    writing_submission::validate_scope(&record, command.manuscript()).map_err(|_| {
        SqlImmediateFailure::Invalid(Box::new(diagnostic(
            command.operation(),
            fence,
            "writing-prepare-scope",
            "writing_context_scope_invalid",
        )))
    })?;
    crate::agent::skills::steering::freeze_grants_sql(tx, &source.store, &record).map_err(
        |cause| match cause {
            E::Sql(cause) => {
                receipts::storage(command.operation(), fence, "writing-prepare-grants", &cause)
            }
            E::Domain(_) => SqlImmediateFailure::Unavailable(Box::new(diagnostic(
                command.operation(),
                fence,
                "writing-prepare-grants",
                "writing_grants_unavailable",
            ))),
        },
    )?;
    writing_submission::freeze_compaction_in(tx, &record, command.compact_revision)
        .map_err(|cause| materialization_error(command, fence, cause))?;
    Ok(record)
}
