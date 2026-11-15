//! Original manual claim transactions and standalone evidence-link publication.
pub(crate) mod commands;
mod error;
mod repository;
mod resources;
use crate::{
    ai::structured_resource_validation::Flow,
    hbs_storage::boundary::{self, BoundaryError},
};
pub(crate) use error::{ClaimsError, EvidenceWriteDiagnostic};
use repository::{LinkProgress, link_evidence_in};
pub(crate) use repository::{claim_from, save_claim_in};
use research_workbench_native_sdk::evidence_write::*;
pub(crate) use resources::ClaimsSql;
use rusqlite::{Connection, Transaction};

pub(crate) trait ClaimsPolicy {
    fn validate(&mut self, input: EvidenceWriteInputV1) -> Result<(), ClaimsError>;
}
#[derive(Clone)]
pub(crate) enum ClaimsRequest {
    Save {
        draft: Box<ClaimDraft>,
    },
    Link {
        draft: Box<EvidenceLinkDraft>,
    },
    Create {
        claim: Box<ClaimDraft>,
        evidence: NewEvidenceDraft,
    },
}
#[derive(Debug)]
pub(crate) enum ClaimsResult {
    Claim(Box<Claim>),
    Link(Box<EvidenceLink>),
}
pub(crate) fn execute(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    db: &mut Connection,
    request: ClaimsRequest,
) -> Result<ClaimsResult, ClaimsError> {
    match request {
        ClaimsRequest::Save { draft } => save_claim_from(sql, policy, true, db, *draft)
            .map(Box::new)
            .map(ClaimsResult::Claim),
        ClaimsRequest::Create { claim, evidence } => {
            create_claim_from(sql, policy, true, db, *claim, evidence)
                .map(Box::new)
                .map(ClaimsResult::Claim)
        }
        ClaimsRequest::Link { draft } => link_evidence_from(sql, policy, true, db, *draft)
            .map(Box::new)
            .map(ClaimsResult::Link),
    }
}
pub(crate) fn save_claim_from(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    explicit: bool,
    db: &mut Connection,
    draft: ClaimDraft,
) -> Result<Claim, ClaimsError> {
    claim_transaction(sql, policy, explicit, db, draft, None)
}
pub(crate) fn create_claim_from(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    explicit: bool,
    db: &mut Connection,
    claim: ClaimDraft,
    evidence: NewEvidenceDraft,
) -> Result<Claim, ClaimsError> {
    claim_transaction(sql, policy, explicit, db, claim, Some(evidence))
}
fn claim_transaction(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    explicit: bool,
    db: &mut Connection,
    draft: ClaimDraft,
    evidence: Option<NewEvidenceDraft>,
) -> Result<Claim, ClaimsError> {
    let mut identity = draft.id.clone();
    let tx = Transaction::new_unchecked(db, rusqlite::TransactionBehavior::Deferred)?;
    let result = (|| {
        let claim = save_claim_in(sql, policy, &tx, draft, &mut identity)?;
        if let Some(evidence) = evidence {
            link_evidence_in(
                sql,
                policy,
                &mut LinkProgress::default(),
                &tx,
                EvidenceLinkDraft {
                    claim_id: claim.id.clone(),
                    anchor_id: evidence.anchor_id,
                    stance: evidence.stance,
                    rationale: evidence.rationale,
                },
            )?;
        }
        Ok(claim.id)
    })();
    let id =
        settle(db, tx, result, explicit).map_err(|cause| match (explicit, identity, cause) {
            (_, _, ClaimsError::Need(input)) => ClaimsError::Need(input),
            (true, Some(id), cause) => ClaimsError::Identified {
                id,
                cause: Box::new(cause),
            },
            (_, _, cause) => cause,
        })?;
    claim_from(sql, db, &id).map_err(|cause| {
        if explicit {
            ClaimsError::Readback {
                id: Some(id),
                cause: Box::new(cause),
            }
        } else {
            cause
        }
    })
}
pub(crate) fn link_evidence_from(
    sql: &ClaimsSql,
    policy: &mut dyn ClaimsPolicy,
    explicit: bool,
    db: &Connection,
    draft: EvidenceLinkDraft,
) -> Result<EvidenceLink, ClaimsError> {
    let mut progress = LinkProgress::default();
    link_evidence_in(sql, policy, &mut progress, db, draft).map_err(|cause| {
        if !explicit {
            return cause;
        }
        if progress.returned {
            return ClaimsError::Readback {
                id: progress.id,
                cause: Box::new(cause),
            };
        }
        if progress.attempted {
            return ClaimsError::StatementFailure(Box::new(cause));
        }
        cause
    })
}
fn settle<T>(
    db: &Connection,
    tx: Transaction<'_>,
    result: Result<T, ClaimsError>,
    explicit: bool,
) -> Result<T, ClaimsError> {
    let result: Result<T, Flow<ClaimsError, Box<EvidenceWriteInputV1>>> =
        result.map_err(|cause| match cause {
            ClaimsError::Need(input) => Flow::Need(input),
            error => Flow::original(error),
        });
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        explicit,
        true,
        ClaimsError::Database,
        |cause| {
            ClaimsError::Technical(
                crate::runtime_plugins::native::NativeHostError::for_profile(
                    crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
                    "storage_error",
                    cause,
                ),
            )
        },
    );
    match boundary::confirm_with(db, settled, explicit, |cause| {
        ClaimsError::Technical(
            crate::runtime_plugins::native::NativeHostError::for_profile(
                crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
                "storage_error",
                cause,
            ),
        )
    }) {
        Ok(value) => Ok(value),
        Err(BoundaryError::Pause(pause)) => Err(ClaimsError::Need(pause.need())),
        Err(BoundaryError::Failure(failure)) if explicit => {
            Err(ClaimsError::Settled(Box::new(failure)))
        }
        Err(BoundaryError::Failure(failure)) => Err(failure.failure.error),
    }
}
