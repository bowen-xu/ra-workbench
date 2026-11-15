//! Proof of the exact context queue start intent, committed with admission.
use super::*;
use crate::agent::context_session::start::Request;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};

const KIND: &str = "agent.context.queue.admission";

// Version 1 must retain its original wire representation even if the public
// request DTO later acquires fields or different serialization attributes.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenQueueV1 {
    input_id: Uuid,
    expected_revision: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FrozenRequestV1 {
    conversation_id: Uuid,
    request_id: Uuid,
    context_snapshot_id: Uuid,
    queue: Option<FrozenQueueV1>,
    resume_queue: bool,
}

fn identity(request: &Request) -> Result<ReceiptIdentity, SqlImmediateFailure> {
    let Request {
        conversation_id,
        request_id,
        context_snapshot_id,
        queue,
        resume_queue,
    } = request;
    let frozen = FrozenRequestV1 {
        conversation_id: *conversation_id,
        request_id: *request_id,
        context_snapshot_id: *context_snapshot_id,
        queue: queue.map(|confirmation| {
            let crate::agent::queue_contract::Confirmation {
                input_id,
                expected_revision,
            } = confirmation;
            FrozenQueueV1 {
                input_id,
                expected_revision,
            }
        }),
        resume_queue: *resume_queue,
    };
    let bytes = serde_json::to_vec(&frozen).map_err(|_| {
        SqlImmediateFailure::Domain(research_agent_core::conversation::CatalogError::InvalidInput)
    })?;
    let mut digest = Sha256::new();
    digest.update(b"agent.context.queue.admission:1:");
    digest.update(bytes);
    Ok(ReceiptIdentity {
        operation_id: Uuid::new_v5(&request.request_id, b"agent.context.queue.admission:v1"),
        kind: KIND,
        version: 1,
        fingerprint: digest.finalize().into(),
    })
}

pub(crate) fn check_context_queue_admission(
    db: &Connection,
    fence: ReadFence,
    request: &Request,
) -> Result<bool, SqlImmediateFailure> {
    let identity = identity(request)?;
    receipts::check_state(db, identity.operation_id, fence, "context-queue-admission")?;
    receipts::check_cross_kind(db, identity.operation_id, fence, "context-queue-admission")?;
    Ok(receipts::lookup(db, &identity, fence, "context-queue-admission")?.is_some())
}

pub(crate) fn insert_context_queue_admission(
    tx: &Transaction<'_>,
    fence: ReadFence,
    request: &Request,
) -> Result<(), SqlImmediateFailure> {
    receipts::insert(tx, &identity(request)?, fence).map(|_| ())
}

pub(crate) fn has_legacy_admission(db: &Connection, request: &Request) -> rusqlite::Result<bool> {
    db.query_row(
        "select 1 from agent_input_queue_admissions where conversation_id=?1 and request_id=?2",
        [
            request.conversation_id.to_string(),
            request.request_id.to_string(),
        ],
        |_| Ok(()),
    )
    .optional()
    .map(|row| row.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::queue_contract::Confirmation;

    #[test]
    fn v1_receipt_retains_its_original_serialized_request_and_policy() {
        let request = Request {
            conversation_id: Uuid::from_u128(1),
            request_id: Uuid::from_u128(2),
            context_snapshot_id: Uuid::from_u128(3),
            queue: Some(Confirmation {
                input_id: Uuid::from_u128(4),
                expected_revision: 9,
            }),
            resume_queue: true,
        };
        let expected = b"{\"conversationId\":\"00000000-0000-0000-0000-000000000001\",\"requestId\":\"00000000-0000-0000-0000-000000000002\",\"contextSnapshotId\":\"00000000-0000-0000-0000-000000000003\",\"queue\":{\"inputId\":\"00000000-0000-0000-0000-000000000004\",\"expectedRevision\":9},\"resumeQueue\":true}";
        let mut hash = Sha256::new();
        hash.update(b"agent.context.queue.admission:1:");
        hash.update(expected);
        let identity = identity(&request).unwrap();
        assert_eq!(identity.fingerprint, <[u8; 32]>::from(hash.finalize()));
        let mut changed = request.clone();
        changed.resume_queue = false;
        assert_ne!(
            identity.fingerprint,
            super::identity(&changed).unwrap().fingerprint
        );
    }
}
