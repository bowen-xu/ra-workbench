use super::*;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResultSnapshot {
    queue: String,
    outcome: TurnOutcome,
}

pub(crate) fn settle_in(
    tx: &Transaction<'_>,
    intent: &SettleIntent,
) -> Result<SettleState, SqlMutationError> {
    let command = settle_command(intent)?;
    authorize(tx, intent.conversation_id, intent.manuscript_id)?;
    let old:Option<(String,String,String)>=tx.query_row("select conversation_id,command_json,result_json from agent_input_queue_receipts where request_id=?1",[intent.operation_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    if let Some((scope, previous, json)) = old {
        if scope != intent.conversation_id.to_string() || previous != command {
            return Err(CatalogError::Conflict.into());
        }
        let saved: ResultSnapshot =
            serde_json::from_str(&json).map_err(|_| CatalogError::Corrupt)?;
        return Ok(SettleState {
            queue: Snapshot::decode(&saved.queue, intent.conversation_id).map_err(domain)?,
            outcome: saved.outcome,
        });
    }
    let (claim, frozen) = read::preparation(
        tx,
        intent.conversation_id,
        intent.input_id,
        intent.request_id,
    )?
    .ok_or(CatalogError::Conflict)?;
    if claim.manuscript_id != intent.manuscript_id
        || claim.expected_queue_revision != intent.claim_revision
    {
        return Err(CatalogError::Conflict.into());
    }
    let record = saved(tx, intent.conversation_id, intent.request_id)?;
    let expected = crate::agent::writing_submission::read_compaction_revision_in(
        tx,
        intent.conversation_id,
        intent.request_id,
    )
    .map_err(writing)?;
    if expected != Some(claim.expected_revision) {
        return Err(CatalogError::Corrupt.into());
    }
    if current_request(tx, intent.conversation_id, intent.input_id)? != Some(intent.request_id) {
        return Err(CatalogError::Conflict.into());
    }
    let q = queue(tx, intent.conversation_id)?;
    let original = frozen
        .inputs()
        .iter()
        .find(|input| input.id == intent.input_id)
        .ok_or(CatalogError::Corrupt)?;
    if q.preparing() != Some(intent.input_id)
        || q.inputs().iter().find(|input| input.id == intent.input_id) != Some(original)
    {
        return Err(CatalogError::Conflict.into());
    }
    let outcome = read::terminal(tx, &record)?.ok_or(CatalogError::Busy)?;
    let edit = if outcome == TurnOutcome::Completed {
        QueueEdit::Admitted(intent.input_id)
    } else {
        QueueEdit::Release(intent.input_id)
    };
    let next = q
        .apply(intent.expected_queue_revision, edit)
        .map_err(|error| domain(error.into()))?;
    let changed=tx.execute("delete from agent_input_queue_previews where input_id=?1 and conversation_id=?2 and request_id=?3",params![intent.input_id.to_string(),intent.conversation_id.to_string(),intent.request_id.to_string()])?;
    if changed != 1 {
        return Err(CatalogError::Conflict.into());
    }
    let json = save_queue(tx, &next)?;
    let result = serde_json::to_string(&ResultSnapshot {
        queue: json,
        outcome,
    })
    .map_err(|_| CatalogError::Corrupt)?;
    tx.execute("insert into agent_input_queue_receipts(request_id,conversation_id,command_json,result_json) values(?1,?2,?3,?4)",params![intent.operation_id.to_string(),intent.conversation_id.to_string(),command,result])?;
    Ok(SettleState {
        queue: next,
        outcome,
    })
}
