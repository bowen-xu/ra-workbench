//! Valid completed history, context windows and a genuinely authorized rewrite.
use super::*;
use crate::{
    agent::repository::{context_windows, input_queue},
    data_interface::AuthorizeRollback,
};
use research_agent_core::{
    context::{
        checkpoint::ContextCheckpoint,
        compaction_strategy::{
            CompactionBinding, CompactionKind, CompactionOutput, CompactionPhase,
        },
    },
    conversation::{BindingKind, ConversationBinding},
    input_queue::{QueueEdit, QueuedInput},
    protocol::{ModelIdentity, TaskKind, TurnOutcome},
};

pub(super) struct Fixture {
    pub a: ConversationId,
    pub b: ConversationId,
    pub first: ContextCheckpoint,
    pub second: ContextCheckpoint,
    pub head: Uuid,
    pub tail: Uuid,
    pub queue_revision: u64,
    pub rollback: JournalCommand,
    pub suffix: Vec<JournalCommand>,
    pub originals: Vec<(JournalCommand, research_data_runtime::WriteReceipt<u64>)>,
    pub items: [Uuid; 2],
}
fn append(
    database: &SharedDatabase,
    originals: &mut Vec<(JournalCommand, research_data_runtime::WriteReceipt<u64>)>,
    command: JournalCommand,
) {
    let receipt = database
        .write(command.clone(), WriteOptions::default())
        .unwrap();
    originals.push((command, receipt));
}
fn checkpoint(
    id: ConversationId,
    turn: Uuid,
    model: &ModelIdentity,
    watermark: u64,
    previous: Option<Uuid>,
) -> ContextCheckpoint {
    ContextCheckpoint {
        schema_version: 1,
        window_id: Uuid::new_v4(),
        conversation_id: *id.as_uuid(),
        turn_id: turn,
        source_watermark: watermark,
        previous_window_id: previous,
        binding: CompactionBinding {
            model: model.clone(),
            strategy_id: "fixture.text".into(),
            state_schema: 1,
            kind: CompactionKind::TextSummary,
        },
        credential_scope_id: None,
        phase: CompactionPhase::Manual,
        output: CompactionOutput::TextSummary("Surviving evidence".into()),
        retained: vec![],
        canonical_context: None,
    }
}
fn turn_started(id: ConversationId, seq: u64, turn: Uuid, model: &ModelIdentity) -> AgentEvent {
    event(
        id,
        seq,
        EventPayload::TurnStarted {
            task_kind: TaskKind::Regular,
            turn_id: turn,
            context_snapshot_id: Uuid::new_v4(),
            model: model.clone(),
        },
    )
}
fn finished_user(id: ConversationId, after: u64, turn: Uuid, text: &str) -> Vec<AgentEvent> {
    let item = Uuid::new_v4();
    vec![
        event(
            id,
            after + 1,
            EventPayload::ItemStarted {
                turn_id: turn,
                item_id: item,
                kind: ItemKind::UserMessage,
            },
        ),
        delta(id, after + 2, turn, item, text),
        event(
            id,
            after + 3,
            EventPayload::ItemCompleted {
                turn_id: turn,
                item_id: item,
            },
        ),
        event(
            id,
            after + 4,
            EventPayload::TurnFinished {
                turn_id: turn,
                outcome: TurnOutcome::Completed,
            },
        ),
    ]
}
pub(super) fn setup(database: &SharedDatabase) -> Fixture {
    let a = ConversationId::from_uuid(Uuid::new_v4());
    let b = ConversationId::from_uuid(Uuid::new_v4());
    let t1 = Uuid::new_v4();
    let t2 = Uuid::new_v4();
    let tb = Uuid::new_v4();
    let items = [Uuid::new_v4(), Uuid::new_v4()];
    let model = ModelIdentity {
        provider_id: "fixture".into(),
        model_id: "model".into(),
        revision: None,
    };
    let mut originals = vec![];
    append(
        database,
        &mut originals,
        command(
            a,
            0,
            vec![turn_started(a, 1, t1, &model)],
            Some("Completed rewrite owner"),
        ),
    );
    let first = checkpoint(a, t1, &model, 1, None);
    context_windows::install(&mut database.lock().unwrap(), &first).unwrap();
    append(
        database,
        &mut originals,
        command(a, 1, finished_user(a, 1, t1, "first evidence"), None),
    );
    append(
        database,
        &mut originals,
        command(a, 5, vec![turn_started(a, 6, t2, &model)], None),
    );
    let second = checkpoint(a, t2, &model, 6, Some(first.window_id));
    context_windows::install(&mut database.lock().unwrap(), &second).unwrap();
    append(
        database,
        &mut originals,
        command(a, 6, finished_user(a, 6, t2, "second evidence"), None),
    );
    append(
        database,
        &mut originals,
        command(
            b,
            0,
            vec![
                turn_started(b, 1, tb, &model),
                start(b, 2, tb, items[0]),
                start(b, 3, tb, items[1]),
            ],
            Some("Other pending owner"),
        ),
    );
    let head = Uuid::new_v4();
    let tail = Uuid::new_v4();
    let queue_revision = {
        let mut db = database.lock().unwrap();
        db.execute("insert into note_pages(id,title,created_at,updated_at) values('coalescer-rollback-note','Synthetic host','now','now')",[]).unwrap();
        db.execute("insert into agent_conversation_bindings(conversation_id,resource_kind,resource_id) values(?1,'note','coalescer-rollback-note')",[a.as_uuid().to_string()]).unwrap();
        for (revision, edit) in [
            QueueEdit::Add(QueuedInput {
                id: head,
                text: "preserved head".into(),
                user_input: vec![],
                origin: None,
            }),
            QueueEdit::Add(QueuedInput {
                id: tail,
                text: "preserved tail".into(),
                user_input: vec![],
                origin: None,
            }),
            QueueEdit::PauseInput(tail),
            QueueEdit::Prepare(head),
        ]
        .into_iter()
        .enumerate()
        {
            input_queue::apply_checked(
                &mut db,
                *a.as_uuid(),
                Uuid::new_v4(),
                revision as u64,
                edit,
                |_| Ok(()),
            )
            .unwrap();
        }
        input_queue::read_checked(&mut db, *a.as_uuid(), |_| Ok(()))
            .unwrap()
            .revision()
    };
    let authorization = AuthorizeRollback {
        operation_id: Uuid::new_v4(),
        conversation_id: a,
        binding: ConversationBinding {
            kind: BindingKind::Note,
            resource_id: "coalescer-rollback-note".into(),
        },
    };
    let proof = database.authorize_rollback(&authorization).unwrap();
    assert!(!proof.replayed);
    let marker = AgentEvent::new(
        a,
        11,
        0,
        authorization.operation_id,
        EventPayload::ConversationRolledBack {
            turn_id: t2,
            user_turns: 1,
        },
    )
    .unwrap();
    let rollback = command(a, 10, vec![marker], None);
    let suffix = vec![
        command(b, 3, vec![delta(b, 4, tb, items[0], "你\0")], None),
        command(b, 4, vec![delta(b, 5, tb, items[1], "B1")], None),
        command(b, 5, vec![delta(b, 6, tb, items[0], "🙂")], None),
        rollback.clone(),
        command(b, 6, vec![delta(b, 7, tb, items[1], "B2")], None),
    ];
    Fixture {
        a,
        b,
        first,
        second,
        head,
        tail,
        queue_revision,
        rollback,
        suffix,
        originals,
        items,
    }
}

pub(super) fn snapshot(db: &Connection) -> Vec<Vec<String>> {
    [
        "select json_array(id,durable_count,title) from agent_conversations order by id",
        "select json_array(conversation_id,item_id,turn_id,body,last_ordinal,completed) from agent_messages order by conversation_id,item_id",
        "select json_array(conversation_id,ordinal,payload_json) from agent_journal_records order by conversation_id,ordinal",
        "select json_array(operation_id,epoch,position,hex(fingerprint),result_json) from data_operation_receipts order by position",
        "select json_array(operation_id,epoch,kind,version,hex(fingerprint),sql_fence_position,result_json) from data_sql_operation_receipts order by operation_id",
        "select json_array(conversation_id,batch_id,expected_count,result_count,payload_hash) from agent_journal_batches order by conversation_id,batch_id",
        "select json_array(conversation_id,batch_id,operation_id) from data_journal_operation_ids order by conversation_id,batch_id",
        "select json_array(materialized_position,coalesce_text) from data_runtime_state cross join data_projection_control",
        "select json_array(conversation_id,window_id) from agent_active_context_windows order by conversation_id",
        "select json_array(id,conversation_id,turn_id,source_watermark,previous_window_id,schema_version,checkpoint_json) from agent_context_checkpoints order by id",
        "select json_array(conversation_id,revision,snapshot_json) from agent_input_queues order by conversation_id",
        "select json_array(conversation_id,window_id) from agent_compact_session_start order by conversation_id",
        "select json_array(conversation_id,turn_id,rollback_ordinal) from agent_rolled_back_turns order by conversation_id,turn_id",
        "select json_array(request_id,conversation_id,resource_kind,resource_id) from agent_rollback_authority order by request_id",
    ].into_iter().map(|query|db.prepare(query).unwrap().query_map([],|row|row.get(0)).unwrap().collect::<Result<_,_>>().unwrap()).collect()
}
