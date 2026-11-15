//! Read the active claim and its frozen input together, without materializing files.
use super::*;
use crate::agent::repository::submissions;
use research_agent_core::submission::SubmissionRecord;

/// Returns only the currently claimed preview, never a canceled/admitted receipt.
/// Authorization runs even when no preview exists. The latest queue revision is
/// returned so later tail edits do not leave recovery using an obsolete receipt.
pub fn read_pending_preview_checked(
    db: &mut Connection,
    conversation: Uuid,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<Option<(SubmissionRecord, InputQueue)>, Error> {
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    authorize(&tx)?;
    let queue = load(&tx, conversation)?;
    let Some(input_id) = queue.preparing() else {
        return Ok(None);
    };
    let request: Option<String> = tx.query_row(
        "select request_id from agent_input_queue_previews where conversation_id=?1 and input_id=?2",
        params![conversation.to_string(), input_id.to_string()], |row| row.get(0))
        .optional().map_err(|_| Error::Storage)?;
    let request = Uuid::parse_str(&request.ok_or(Error::Corrupt)?).map_err(|_| Error::Corrupt)?;
    if crate::agent::writing_submission::read_compaction_revision_in(&tx, conversation, request)
        .map_err(|_| Error::Corrupt)?
        .is_some()
    {
        // A typed compact claim is recovered by its own purpose-aware reader.
        return Ok(None);
    }
    let admitted: bool = tx
        .query_row(
            "select exists(select 1 from agent_input_queue_admissions where input_id=?1)",
            [input_id.to_string()],
            |row| row.get(0),
        )
        .map_err(|_| Error::Storage)?;
    if admitted {
        return Err(Error::Corrupt);
    }
    let record = submissions::read(&tx, conversation, request)
        .map_err(admission::submission_error)?
        .ok_or(Error::Corrupt)?;
    if queue
        .inputs()
        .first()
        .is_none_or(|head| head.id != input_id || head.text != record.input.text)
    {
        return Err(Error::Corrupt);
    }
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(Some((record, queue)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::create_conversation;
    use research_agent_core::{
        conversation::ConversationId, protocol::ModelIdentity, submission::SubmissionInput,
    };
    fn fixture() -> (Connection, Uuid, Uuid, Uuid, SubmissionInput) {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        let scope = Uuid::new_v4();
        let input_id = Uuid::new_v4();
        let request = Uuid::new_v4();
        create_conversation(&mut db, ConversationId::from_uuid(scope), "recovery").unwrap();
        let input = SubmissionInput {
            remote_compaction_v2: None,
            user_input: vec![],
            skill_instructions: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
            text: "frozen".into(),
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
                id: input_id,
                text: input.text.clone(),
            }),
            |_| Ok(()),
        )
        .unwrap();
        (db, scope, input_id, request, input)
    }
    #[test]
    fn recovery_uses_current_revision_and_keeps_the_frozen_submission() {
        let (mut db, scope, id, request, input) = fixture();
        assert!(
            read_pending_preview_checked(&mut db, scope, |_| Ok(()))
                .unwrap()
                .is_none()
        );
        let (saved, _) =
            prepare_preview_checked(&mut db, scope, request, 1, id, input, |_| Ok(())).unwrap();
        apply_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            2,
            QueueEdit::Add(QueuedInput {
                user_input: vec![],
                origin: None,
                id: Uuid::new_v4(),
                text: "tail".into(),
            }),
            |_| Ok(()),
        )
        .unwrap();
        let (restored, queue) = read_pending_preview_checked(&mut db, scope, |_| Ok(()))
            .unwrap()
            .unwrap();
        assert_eq!(saved, restored);
        assert_eq!(queue.revision(), 3);
        assert_eq!(queue.inputs().len(), 2);
        cancel_preview_checked(
            &mut db,
            scope,
            Uuid::new_v4(),
            queue.revision(),
            id,
            request,
            |_| Ok(()),
        )
        .unwrap();
        assert!(
            read_pending_preview_checked(&mut db, scope, |_| Ok(()))
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn admitted_previews_are_not_recovered_and_revoked_access_is_rechecked() {
        let (mut db, scope, id, request, input) = fixture();
        prepare_preview_checked(&mut db, scope, request, 1, id, input.clone(), |_| Ok(())).unwrap();
        assert!(matches!(
            read_pending_preview_checked(&mut db, scope, |_| Err(Error::Closed)),
            Err(Error::Closed)
        ));
        admit_checked(&mut db, scope, request, 2, id, input, |_| Ok(())).unwrap();
        assert!(
            read_pending_preview_checked(&mut db, scope, |_| Ok(()))
                .unwrap()
                .is_none()
        );
        assert!(matches!(
            read_pending_preview_checked(&mut db, scope, |_| Err(Error::Closed)),
            Err(Error::Closed)
        ));
    }
    #[test]
    fn missing_preview_link_is_corruption_not_an_empty_queue() {
        let (mut db, scope, id, request, input) = fixture();
        prepare_preview_checked(&mut db, scope, request, 1, id, input, |_| Ok(())).unwrap();
        db.execute(
            "delete from agent_input_queue_previews where input_id=?1",
            [id.to_string()],
        )
        .unwrap();
        assert!(matches!(
            read_pending_preview_checked(&mut db, scope, |_| Ok(())),
            Err(Error::Corrupt)
        ));
        assert_eq!(
            read_checked(&mut db, scope, |_| Ok(()))
                .unwrap()
                .preparing(),
            Some(id)
        );
    }
}
