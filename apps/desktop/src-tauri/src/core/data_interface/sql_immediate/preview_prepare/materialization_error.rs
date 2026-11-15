//! Preserve known safe preparation causes without paths or source contents.
use super::*;

enum FailureClass {
    Invalid,
    Conflict,
    Unavailable,
    Corrupt,
}

pub(super) fn materialization_error(
    command: &QueuePreviewPrepareIntent,
    fence: ReadFence,
    cause: queue::PrepareError,
) -> SqlImmediateFailure {
    use crate::agent::writing_context::WritingContextError as C;
    use crate::agent::{
        instructions::InstructionReadError as I, writing_submission::WritingSubmissionError as W,
    };
    use FailureClass::{Conflict, Corrupt, Invalid, Unavailable};
    use research_agent_core::submission::SubmissionError as S;
    let (class, phase, reason) = match cause {
        queue::PrepareError::Mutation(cause) => {
            return error(command, fence, "queue-prepare", cause);
        }
        queue::PrepareError::Grants => (
            Unavailable,
            "queue-prepare-grants",
            "writing_grants_unavailable",
        ),
        queue::PrepareError::Materialization(cause) => match *cause {
            W::Storage(cause) => return cause.into_inner(),
            W::Sql(cause) => {
                return receipts::storage(
                    command.request_id,
                    fence,
                    "queue-prepare-materialize",
                    &cause,
                );
            }
            W::Unbound => {
                return SqlImmediateFailure::Domain(
                    research_agent_core::conversation::CatalogError::NotFound,
                );
            }
            W::Context(C::Conflict) => {
                (Conflict, "queue-prepare-context", "writing_context_changed")
            }
            W::Context(C::InvalidInput) => {
                (Invalid, "queue-prepare-context", "writing_context_invalid")
            }
            W::Context(C::Budget) => (
                Invalid,
                "queue-prepare-context",
                "writing_context_budget_exceeded",
            ),
            W::Context(C::Unavailable) => (
                Unavailable,
                "queue-prepare-context",
                "writing_context_source_unavailable",
            ),
            W::Instructions(I::Unauthorized) => (
                Conflict,
                "queue-prepare-instructions",
                "writing_instructions_unauthorized",
            ),
            W::Instructions(I::InvalidScope) => (
                Invalid,
                "queue-prepare-instructions",
                "writing_instructions_scope_invalid",
            ),
            W::Instructions(I::Budget) => (
                Invalid,
                "queue-prepare-instructions",
                "writing_instructions_budget_exceeded",
            ),
            W::Instructions(I::Encoding) => (
                Invalid,
                "queue-prepare-instructions",
                "writing_instructions_encoding_invalid",
            ),
            W::Instructions(I::UnsupportedPlatform) => (
                Unavailable,
                "queue-prepare-instructions",
                "writing_instructions_platform_unsupported",
            ),
            W::Instructions(I::Unavailable) => (
                Unavailable,
                "queue-prepare-instructions",
                "writing_instructions_unavailable",
            ),
            W::Input(S::Conflict) => (Conflict, "queue-prepare-input", "writing_input_changed"),
            W::Input(S::InvalidInput) => (Invalid, "queue-prepare-input", "writing_input_invalid"),
            W::Input(S::Corrupt) => (Corrupt, "queue-prepare-input", "writing_input_corrupt"),
            W::Input(_) => (
                Unavailable,
                "queue-prepare-input",
                "writing_input_unavailable",
            ),
            W::Model(_) => (
                Unavailable,
                "queue-prepare-model",
                "writing_model_preparation_unavailable",
            ),
            // Native waits run in the owned preparation boundary, outside this
            // queue SQL materializer. A synchronous source returns Need instead.
            W::Native(_) | W::Unavailable => (
                Unavailable,
                "queue-prepare-materialize",
                "writing_preparation_unavailable",
            ),
        },
    };
    let detail = Box::new(diagnostic(command.request_id, fence, phase, reason));
    match class {
        Invalid => SqlImmediateFailure::Invalid(detail),
        Conflict => SqlImmediateFailure::Conflict(detail),
        Corrupt => SqlImmediateFailure::Corrupt(detail),
        Unavailable => SqlImmediateFailure::Unavailable(detail),
    }
}
