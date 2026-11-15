//! Recovery candidates for the gap between queue admission and owner startup.
use super::*;
use crate::agent::repository::submissions;
use research_agent_core::{
    protocol::{AgentEvent, EventPayload},
    submission::SubmissionRecord,
};

pub struct UnstartedQueueAdmission {
    pub input_id: Uuid,
    pub expected_revision: u64,
    pub submission: SubmissionRecord,
}
/// A read-only candidate list, not permission to restart execution. The owner must
/// recheck durable history when a user retries. Cursor order is input identity,
/// not execution priority. A full page may be followed by an empty final page.
pub fn list_unstarted_checked(
    db: &mut Connection,
    conversation: Uuid,
    after: Option<Uuid>,
    limit: u8,
    authorize: impl FnOnce(&Connection) -> Result<(), Error>,
) -> Result<Vec<UnstartedQueueAdmission>, Error> {
    if limit == 0 || limit > 64 || after.is_some_and(|id| id.is_nil()) {
        return Err(QueueError::Invalid.into());
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| Error::Storage)?;
    require_open(&tx, conversation)?;
    authorize(&tx)?;
    let mut statement = tx.prepare(
        "select a.input_id,a.expected_revision,a.request_id from agent_input_queue_admissions a
         join agent_submissions s on s.conversation_id=a.conversation_id and s.request_id=a.request_id
         where a.conversation_id=?1 and (?2 is null or a.input_id>?2)
         order by a.input_id limit ?3").map_err(|_| Error::Storage)?;
    let rows = statement
        .query_map(
            params![
                conversation.to_string(),
                after.map(|id| id.to_string()),
                limit
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|_| Error::Storage)?;
    let mut result = Vec::new();
    for row in rows {
        let (id, expected_revision, request) = row.map_err(|_| Error::Storage)?;
        let input_id = Uuid::parse_str(&id).map_err(|_| Error::Corrupt)?;
        let request = Uuid::parse_str(&request).map_err(|_| Error::Corrupt)?;
        let submission = submissions::read(&tx, conversation, request)
            .map_err(admission::submission_error)?
            .ok_or(Error::Corrupt)?;
        if crate::agent::writing_submission::read_compaction_revision_in(&tx, conversation, request)
            .map_err(|_| Error::Corrupt)?
            .is_some()
        {
            continue;
        }
        if has_matching_turn_started(&tx, conversation, &submission)? {
            continue;
        }
        result.push(UnstartedQueueAdmission {
            input_id,
            expected_revision,
            submission,
        });
    }
    drop(statement);
    tx.commit().map_err(|_| Error::Storage)?;
    Ok(result)
}

fn has_matching_turn_started(
    db: &Connection,
    conversation: Uuid,
    submission: &SubmissionRecord,
) -> Result<bool, Error> {
    let mut statement = db
        .prepare("select payload_json from agent_items where conversation_id=?1 order by ordinal")
        .map_err(|_| Error::Storage)?;
    let rows = statement
        .query_map([conversation.to_string()], |row| row.get::<_, String>(0))
        .map_err(|_| Error::Storage)?;
    for row in rows {
        let json = row.map_err(|_| Error::Storage)?;
        let event: AgentEvent = serde_json::from_str(&json).map_err(|_| Error::Corrupt)?;
        let EventPayload::TurnStarted {
            turn_id,
            context_snapshot_id,
            model,
            ..
        } = event.payload()
        else {
            continue;
        };
        if *turn_id != submission.turn_id {
            continue;
        }
        if *context_snapshot_id != submission.context_snapshot_id
            || *model != submission.input.model
        {
            return Err(Error::Corrupt);
        }
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::repository::create_conversation;
    use research_agent_core::{
        conversation::ConversationId,
        protocol::{AgentEvent, EventPayload, ModelIdentity},
        submission::SubmissionInput,
    };
    #[test]
    fn reopened_admission_can_be_found_but_started_turns_are_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queue.sqlite");
        let mut db = Connection::open(&path).unwrap();
        crate::storage_migration_tests::migrate_through(&mut db, 94);
        let scope = Uuid::new_v4();
        create_conversation(&mut db, ConversationId::from_uuid(scope), "recovery").unwrap();
        let mut records = Vec::new();
        for index in 1..=3 {
            let input_id = Uuid::from_u128(index);
            let revision = (index as u64 - 1) * 3;
            let input = SubmissionInput {
                remote_compaction_v2: None,
                user_input: vec![],
                skill_instructions: vec![],
                instructions: vec![],
                instruction_configs: vec![],
                skills: vec![],
                text: format!("message {index}"),
                model: ModelIdentity {
                    provider_id: "fixture".into(),
                    model_id: "fixture".into(),
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
                revision,
                QueueEdit::Add(QueuedInput {
                    user_input: vec![],
                    origin: None,
                    id: input_id,
                    text: input.text.clone(),
                }),
                |_| Ok(()),
            )
            .unwrap();
            apply_checked(
                &mut db,
                scope,
                Uuid::new_v4(),
                revision + 1,
                QueueEdit::Prepare(input_id),
                |_| Ok(()),
            )
            .unwrap();
            records.push(
                admit_checked(
                    &mut db,
                    scope,
                    Uuid::new_v4(),
                    revision + 2,
                    input_id,
                    input,
                    |_| Ok(()),
                )
                .unwrap()
                .0,
            );
        }
        drop(db);
        let mut db = Connection::open(path).unwrap();
        let first = list_unstarted_checked(&mut db, scope, None, 1, |_| Ok(())).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].submission, records[0]);
        assert_eq!(first[0].expected_revision, 2);
        let next =
            list_unstarted_checked(&mut db, scope, Some(first[0].input_id), 1, |_| Ok(())).unwrap();
        assert_eq!(next[0].submission, records[1]);
        let event = AgentEvent::new(
            ConversationId::from_uuid(scope),
            1,
            0,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: records[0].turn_id,
                context_snapshot_id: records[0].context_snapshot_id,
                model: records[0].input.model.clone(),
            },
        )
        .unwrap();
        db.execute("insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,0,1,?2)",
            params![scope.to_string(),serde_json::to_string(&event).unwrap()]).unwrap();
        let remaining = list_unstarted_checked(&mut db, scope, None, 64, |_| Ok(())).unwrap();
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0].submission, records[1]);
        let mismatched = AgentEvent::new(
            ConversationId::from_uuid(scope),
            2,
            0,
            Uuid::new_v4(),
            EventPayload::TurnStarted {
                task_kind: research_agent_core::protocol::TaskKind::Regular,
                turn_id: records[1].turn_id,
                context_snapshot_id: Uuid::new_v4(),
                model: ModelIdentity {
                    provider_id: "wrong-provider".into(),
                    model_id: "wrong-model".into(),
                    revision: None,
                },
            },
        )
        .unwrap();
        db.execute(
            "insert into agent_items(conversation_id,ordinal,schema_version,payload_json) values(?1,1,1,?2)",
            params![scope.to_string(), serde_json::to_string(&mismatched).unwrap()],
        )
        .unwrap();
        assert!(matches!(
            list_unstarted_checked(&mut db, scope, None, 64, |_| Ok(())),
            Err(Error::Corrupt)
        ));
        assert!(matches!(
            list_unstarted_checked(&mut db, scope, None, 1, |_| Err(Error::Closed)),
            Err(Error::Closed)
        ));
        assert!(list_unstarted_checked(&mut db, scope, None, 0, |_| Ok(())).is_err());
        assert!(
            read_checked(&mut db, scope, |_| Ok(()))
                .unwrap()
                .inputs()
                .is_empty()
        );
    }
}
