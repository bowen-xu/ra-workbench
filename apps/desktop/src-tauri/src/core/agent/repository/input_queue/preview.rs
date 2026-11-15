//! Freeze a preview and claim its queue head without consuming the input.
use super::*;
use crate::agent::repository::submissions;
use research_agent_core::submission::{SubmissionInput, SubmissionRecord};

pub fn prepare_preview_checked(
    db: &mut Connection,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    authorize(&tx)?;
    let result = prepare_preview_in(&tx, conversation, request, revision, input_id, input)?;
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(result)
}

/// Host owns the transaction and checks binding before materializing input.
pub(crate) fn prepare_preview_in(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
) -> Result<(SubmissionRecord, InputQueue), Error> {
    let user_input = input.user_input.clone();
    prepare_resolved_preview_in(
        tx,
        conversation,
        request,
        revision,
        input_id,
        input,
        &user_input,
    )
}

/// Hosts may resolve composer references to frozen source identities. Validate
/// the original queued payload before storing its immutable resolved submission.
pub(crate) fn prepare_resolved_preview_in(
    tx: &rusqlite::Transaction<'_>,
    conversation: Uuid,
    request: Uuid,
    revision: u64,
    input_id: Uuid,
    input: SubmissionInput,
    queued_user_input: &[research_agent_core::user_input::UserInput],
) -> Result<(SubmissionRecord, InputQueue), Error> {
    if request.is_nil() || input_id.is_nil() {
        return Err(QueueError::Invalid.into());
    }
    input.validate().map_err(admission::submission_error)?;
    require_open(tx, conversation)?;
    let prior: Option<(String,String,u64,String)> = tx.query_row(
        "select conversation_id,request_id,expected_revision,result_json from agent_input_queue_previews where input_id=?1",
        [input_id.to_string()], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))
        .optional().map_err(|_| Error::Storage)?;
    if let Some((scope, old_request, old_revision, json)) = prior {
        if scope != conversation.to_string()
            || old_request != request.to_string()
            || old_revision != revision
        {
            return Err(QueueError::Conflict.into());
        }
        let record = submissions::read(tx, conversation, request)
            .map_err(admission::submission_error)?
            .ok_or(Error::Corrupt)?;
        if record.input != input {
            return Err(QueueError::Conflict.into());
        }
        return Ok((record, Snapshot::decode(&json, conversation)?));
    }
    handoff_guard::require_unclaimed(tx, conversation, input_id)
        .map_err(|cause| cause.domain_or(Error::Storage))?;
    if submissions::read(tx, conversation, request)
        .map_err(admission::submission_error)?
        .is_some()
    {
        return Err(QueueError::Conflict.into());
    }
    let queue = load(tx, conversation)?;
    if queue.next_input().is_none_or(|head| {
        head.id != input_id || head.text != input.text || head.user_input != queued_user_input
    }) {
        return Err(QueueError::Conflict.into());
    }
    let next = queue.apply(revision, QueueEdit::Prepare(input_id))?;
    let record = submissions::insert_in(tx, conversation, request, input)
        .map_err(admission::submission_error)?;
    let json = Snapshot::encode(&next)?;
    tx.execute("insert into agent_queue_submission_origins(conversation_id,request_id,input_id) values(?1,?2,?3)",
        params![conversation.to_string(),request.to_string(),input_id.to_string()]).map_err(|_|Error::Storage)?;
    tx.execute(
        "update agent_input_queues set revision=?2,snapshot_json=?3 where conversation_id=?1",
        params![conversation.to_string(), next.revision(), json],
    )
    .map_err(|_| Error::Storage)?;
    tx.execute("insert into agent_input_queue_previews(input_id,conversation_id,request_id,expected_revision,result_json) values(?1,?2,?3,?4,?5)",
        params![input_id.to_string(),conversation.to_string(),request.to_string(),revision,json]).map_err(|_|Error::Storage)?;
    Ok((record, next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::create_conversation;
    use research_agent_core::{conversation::ConversationId, protocol::ModelIdentity};
    fn fixture() -> (Connection, Uuid, Uuid, SubmissionInput) {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        let scope = Uuid::new_v4();
        let id = Uuid::new_v4();
        create_conversation(&mut db, ConversationId::from_uuid(scope), "preview").unwrap();
        let input = SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            text: "queued text".into(),
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "test".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
        };
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id,
                text: input.text.clone(),
            }),
            |_| Ok(()),
        )
        .unwrap();
        (db, scope, id, input)
    }
    #[test]
    fn preview_retains_head_until_confirmation_and_reuses_frozen_turn() {
        let (mut db, scope, id, input) = fixture();
        let request = Uuid::new_v4();
        let (record, queue) =
            prepare_preview_checked(&mut db, scope, request, 1, id, input.clone(), |_| Ok(()))
                .unwrap();
        assert_eq!(queue.inputs().len(), 1);
        assert_eq!(queue.preparing(), Some(id));
        let (retry, _) =
            prepare_preview_checked(&mut db, scope, request, 1, id, input.clone(), |_| Ok(()))
                .unwrap();
        assert_eq!(record, retry);
        let (admitted, queue) =
            admit_checked(&mut db, scope, request, 2, id, input, |_| Ok(())).unwrap();
        assert_eq!(admitted, record);
        assert!(queue.inputs().is_empty());
    }
    #[test]
    fn preview_write_failure_rolls_back_claim_and_submission() {
        let (mut db, scope, id, input) = fixture();
        let request = Uuid::new_v4();
        db.execute_batch("create trigger fail_preview before insert on agent_input_queue_previews begin select raise(abort,'fixture'); end;").unwrap();
        assert!(
            prepare_preview_checked(&mut db, scope, request, 1, id, input, |_| Ok(())).is_err()
        );
        assert!(submissions::read(&db, scope, request).unwrap().is_none());
        let queue = read_checked(&mut db, scope, |_| Ok(())).unwrap();
        assert_eq!(queue.preparing(), None);
        assert_eq!(queue.revision(), 1);
    }
    #[test]
    fn changed_preview_and_revoked_binding_do_not_reinterpret_original() {
        let (mut db, scope, id, input) = fixture();
        let request = Uuid::new_v4();
        prepare_preview_checked(&mut db, scope, request, 1, id, input.clone(), |_| Ok(())).unwrap();
        let mut changed = input.clone();
        changed.model.model_id = "changed".into();
        assert!(
            prepare_preview_checked(&mut db, scope, request, 1, id, changed, |_| Ok(())).is_err()
        );
        assert!(matches!(
            prepare_preview_checked(&mut db, scope, request, 1, id, input, |_| Err(
                Error::Closed
            )),
            Err(Error::Closed)
        ));
        assert_eq!(
            read_checked(&mut db, scope, |_| Ok(())).unwrap().inputs()[0].text,
            "queued text"
        );
    }
}
