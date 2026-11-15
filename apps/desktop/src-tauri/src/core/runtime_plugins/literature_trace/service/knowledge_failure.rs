//! Shared existing knowledge settlement diagnostics, with domain metadata attached by the caller.
use super::*;
use crate::{
    ai::{
        error::{AiCommandError, AiError},
        knowledge_contracts::{KnowledgeCommandError, KnowledgeError},
    },
    hbs_storage::boundary::HbsOutcome,
    runtime_plugins::native::{SafeDatabaseError, StorageMutationDiagnostic},
};
#[derive(Clone, Copy)]
pub(super) enum KnowledgeMutationKind {
    Relation,
    ConceptIdentity,
    ConceptAlgorithm,
}
pub(super) fn command_error(
    kind: KnowledgeMutationKind,
    error: AiError,
    key: &str,
) -> Box<NativeHostError> {
    match error {
        AiError::NativeWorkflow { failure, .. } => failure,
        error @ (AiError::Database(_) | AiError::Knowledge(KnowledgeError::Database(_))) => {
            mutation_error(
                kind,
                Some(error),
                "execution",
                HbsOutcome::NotStarted,
                key,
                None,
            )
        }
        AiError::Knowledge(error) => KnowledgeCommandError::from(error)
            .into_native()
            .package(key),
        error => {
            let error = AiCommandError::from(error);
            LiteratureTraceService::error(error.code(), error.message()).package(key)
        }
    }
}
pub(super) fn native_ai(failure: Box<NativeHostError>) -> AiError {
    let code = if failure.code == "data_operation_outcome_unknown" {
        "data_operation_outcome_unknown"
    } else {
        "storage_error"
    };
    AiError::NativeWorkflow { code, failure }
}
pub(super) fn mutation_error(
    kind: KnowledgeMutationKind,
    error: Option<AiError>,
    phase: &'static str,
    outcome: HbsOutcome,
    key: &str,
    rollback_error: Option<Box<SafeDatabaseError>>,
) -> Box<NativeHostError> {
    let (database_error, original) = match error {
        Some(AiError::Database(cause) | AiError::Knowledge(KnowledgeError::Database(cause))) => {
            (Some(Box::new(SafeDatabaseError::sql(phase, &cause))), None)
        }
        Some(AiError::NativeWorkflow { failure, .. }) => (None, Some(failure)),
        _ => (None, None),
    };
    let code = if outcome == HbsOutcome::Unknown {
        "data_operation_outcome_unknown"
    } else {
        "storage_error"
    };
    let subject = match kind {
        KnowledgeMutationKind::Relation => "关系",
        KnowledgeMutationKind::ConceptIdentity => "概念身份",
        KnowledgeMutationKind::ConceptAlgorithm => "概念算法",
    };
    let message = match outcome {
        HbsOutcome::Committed => format!("{subject}已提交，结果读取失败，请重新载入。"),
        HbsOutcome::Unknown => format!("{subject}结算结果未知，请重启后核对结果。"),
        _ => format!("{subject}数据保存失败，请核对原因后重试。"),
    };
    let mut failure = match original {
        Some(mut failure) => {
            if outcome == HbsOutcome::Unknown {
                failure.code = code.into();
                failure.message = format!("{message} {}", failure.message);
            }
            failure
        }
        None => LiteratureTraceService::error(code, &message).package(key),
    };
    if failure.package_key.is_none() {
        failure.package_key = Some(key.to_owned());
    }
    if let Some(detail) = failure.concepts.as_mut() {
        use crate::runtime_plugins::literature_concepts::ConceptOutcome;
        detail.outcome = match outcome {
            HbsOutcome::NotStarted => ConceptOutcome::NotStarted,
            HbsOutcome::RolledBack => ConceptOutcome::RolledBack,
            HbsOutcome::Committed => ConceptOutcome::Committed,
            HbsOutcome::Unknown => ConceptOutcome::Unknown,
        };
    }
    let diagnostic = Some(Box::new(StorageMutationDiagnostic {
        phase,
        outcome,
        object_id: None,
        revision: None,
        database_error,
        rollback_error,
    }));
    match kind {
        KnowledgeMutationKind::Relation => failure.relations = diagnostic,
        KnowledgeMutationKind::ConceptIdentity => failure.concept_identity = diagnostic,
        KnowledgeMutationKind::ConceptAlgorithm => {
            failure.concept_algorithm = diagnostic.map(|detail| {
                Box::new(StorageMutationDiagnostic {
                    phase: detail.phase,
                    outcome: detail.outcome.into(),
                    object_id: detail.object_id,
                    revision: detail.revision,
                    database_error: detail.database_error,
                    rollback_error: detail.rollback_error,
                })
            })
        }
    };
    failure
}
