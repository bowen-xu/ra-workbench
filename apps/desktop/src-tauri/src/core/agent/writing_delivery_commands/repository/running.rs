//! Original running delivery proof checks and immutable evidence.
use super::*;

/// Checks the released copy against immutable proposal and application facts.
pub(in crate::agent::writing_delivery_commands) fn running_proof_with<P>(
    db: &Connection,
    request: &Acknowledge,
    review: &crate::agent::writing_submission::workspace::WorkspaceReview,
    delivery_id: Uuid,
    mut read_checkpoint: impl FnMut(
        crate::agent::repository::proposal_applications::ApplicationScope,
        &str,
    ) -> Result<
        Option<crate::agent::writing_checkpoint_contracts::CheckpointRecord>,
        crate::ai::structured_resource_validation::Flow<Error, P>,
    >,
) -> Result<RunningProof, crate::ai::structured_resource_validation::Flow<Error, P>> {
    use crate::agent::repository::{change_sets, proposal_applications as applications};
    use crate::ai::structured_resource_validation::Flow;
    use applications::{ApplicationPhase, ApplicationStatus, ApplicationTarget};
    use sha2::{Digest, Sha256};
    admitted(db, request).map_err(Flow::original)?;
    let scope = &request.scope;
    let proposal: Option<String> = db.query_row(
        "select id from agent_change_sets where conversation_id=?1 and request_id=?2 and manuscript_id=?3 and context_snapshot_id=?4 and turn_id=?5 and lease_id=?6 and manifest_hash=?7 order by created_at desc,id desc limit 1",
        params![scope.conversation_id.to_string(), scope.request_id.to_string(), scope.manuscript_id.to_string(), scope.context_snapshot_id.to_string(), request.turn_id.to_string(), review.binding.lease_id.to_string(), review.binding.manifest_hash], |row| row.get(0)
    ).optional().map_err(|cause| Flow::original(storage(cause)))?;
    let Some(proposal) = proposal else {
        return if review.changes.is_empty() {
            persist_running_proof(
                db,
                request,
                review,
                delivery_id,
                RunningProof::Empty,
                serde_json::json!({"schemaVersion":1,"kind":"empty"}),
            )
            .map_err(Flow::original)
        } else {
            Err(Flow::original(Error::RecoveryRequired))
        };
    };
    let proposal = id(&proposal).map_err(Flow::original)?;
    let frozen = change_sets::read_in(db, scope.conversation_id, scope.manuscript_id, proposal)
        .map_err(|_| Flow::original(Error::Conflict))?;
    if frozen.binding != review.binding
        || frozen.changes != review.changes
        || frozen.binding.request_id != scope.request_id
        || frozen.binding.context_snapshot_id != scope.context_snapshot_id
        || frozen.binding.turn_id != request.turn_id
    {
        return Err(Flow::original(Error::Conflict));
    }
    if frozen.status == change_sets::ChangeSetStatus::Rejected {
        return persist_running_proof(
            db,
            request,
            review,
            delivery_id,
            RunningProof::Rejected,
            serde_json::json!({"schemaVersion":1,"kind":"rejected","proposalId":proposal}),
        )
        .map_err(Flow::original);
    }
    if frozen.status != change_sets::ChangeSetStatus::Applied {
        return Err(Flow::original(Error::RecoveryRequired));
    }
    let application =
        applications::find_in(db, scope.conversation_id, scope.manuscript_id, proposal)
            .map_err(|_| Flow::original(Error::Conflict))?
            .ok_or_else(|| Flow::original(Error::RecoveryRequired))?;
    if application.status != ApplicationStatus::Applied
        || application
            .files
            .iter()
            .any(|file| file.phase != ApplicationPhase::Acknowledged)
    {
        return Err(Flow::original(Error::RecoveryRequired));
    }
    let mut editors = Vec::new();
    for file in &application.files {
        let ApplicationTarget::Editor {
            expected,
            result_revision,
        } = &file.target
        else {
            continue;
        };
        let checkpoint = read_checkpoint(
            crate::agent::repository::proposal_applications::ApplicationScope {
                conversation: scope.conversation_id,
                manuscript: scope.manuscript_id,
                proposal,
                operation: application.operation_id,
            },
            &file.path,
        )?
        .ok_or_else(|| Flow::original(Error::RecoveryRequired))?
        .checkpoint;
        let receipt = checkpoint
            .editor_receipt
            .as_ref()
            .ok_or_else(|| Flow::original(Error::RecoveryRequired))?;
        let after = frozen
            .changes
            .iter()
            .find(|change| change.path == file.path)
            .and_then(|change| change.after.as_ref())
            .ok_or_else(|| Flow::original(Error::Conflict))?;
        let hash = format!("{:x}", Sha256::digest(receipt.text.as_bytes()));
        if receipt.operation_id != application.operation_id.to_string()
            || receipt.session_id != expected.session_id.to_string()
            || result_revision
                .map(|revision| revision.to_string())
                .as_deref()
                != Some(receipt.revision.as_str())
            || after.sha256 != hash
            || after.byte_size != receipt.text.len() as u64
            || checkpoint.saved_version < receipt.state_version.unwrap_or(checkpoint.version)
            || !checkpoint.operations.iter().any(|source| {
                source.operation_id == receipt.operation_id
                    && source.conversation_id == scope.conversation_id.to_string()
                    && source.request_id == scope.request_id.to_string()
                    && source.result_sha256 == hash
            })
        {
            return Err(Flow::original(Error::RecoveryRequired));
        }
        editors.push(serde_json::json!({
            "path": file.path,
            "sessionId": receipt.session_id,
            "revision": receipt.revision,
            "resultSha256": hash,
            "stateVersion": receipt.state_version.unwrap_or(checkpoint.version),
            "savedVersion": checkpoint.saved_version,
            "savedSha256": checkpoint.saved_sha256,
        }));
    }
    persist_running_proof(
        db,
        request,
        review,
        delivery_id,
        RunningProof::Applied,
        serde_json::json!({"schemaVersion":1,"kind":"applied","proposalId":proposal,"operationId":application.operation_id,"editors":editors}),
    ).map_err(Flow::original)
}

fn persist_running_proof(
    db: &Connection,
    request: &Acknowledge,
    review: &crate::agent::writing_submission::workspace::WorkspaceReview,
    delivery_id: Uuid,
    proof: RunningProof,
    evidence: serde_json::Value,
) -> Result<RunningProof, Error> {
    let scope = &request.scope;
    let json = serde_json::to_string(&evidence).map_err(|_| Error::Conflict)?;
    let inserted = db.execute("insert into writing_agent_delivery_proofs(delivery_id,conversation_id,request_id,turn_id,context_snapshot_id,manuscript_id,lease_id,manifest_hash,proof_json) values(?1,?2,?3,?4,?5,?6,?7,?8,?9) on conflict do nothing", params![delivery_id.to_string(), scope.conversation_id.to_string(), scope.request_id.to_string(), request.turn_id.to_string(), scope.context_snapshot_id.to_string(), scope.manuscript_id.to_string(), review.binding.lease_id.to_string(), review.binding.manifest_hash, json]).map_err(storage)?;
    if inserted == 0 {
        let matches: bool = db.query_row("select exists(select 1 from writing_agent_delivery_proofs where delivery_id=?1 and conversation_id=?2 and request_id=?3 and turn_id=?4 and context_snapshot_id=?5 and manuscript_id=?6 and lease_id=?7 and manifest_hash=?8 and proof_json=?9)", params![delivery_id.to_string(), scope.conversation_id.to_string(), scope.request_id.to_string(), request.turn_id.to_string(), scope.context_snapshot_id.to_string(), scope.manuscript_id.to_string(), review.binding.lease_id.to_string(), review.binding.manifest_hash, json], |row| row.get(0)).map_err(storage)?;
        if !matches {
            return Err(Error::Conflict);
        }
    }
    Ok(proof)
}
