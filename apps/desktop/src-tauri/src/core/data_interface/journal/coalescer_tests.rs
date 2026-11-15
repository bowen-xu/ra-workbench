//! Real projector SQL observes interleaved bodies, barriers and canonical proofs.
use super::*;

pub(super) fn count_bodies(db: &Connection) {
    db.execute_batch("create table body_updates(n integer primary key,conversation text,item text,body text,last_ordinal integer);
        create trigger count_body_updates after update of body on agent_messages begin
        insert into body_updates(conversation,item,body,last_ordinal) values(new.conversation_id,new.item_id,new.body,new.last_ordinal); end;").unwrap();
}
fn message(db: &Connection, id: ConversationId, item: Uuid) -> (String, u64, bool) {
    db.query_row("select body,last_ordinal,completed from agent_messages where conversation_id=?1 and item_id=?2",
        params![id.as_uuid().to_string(),item.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap()
}
fn assert_history(
    projector: &SqlProjector,
    records: &[Accepted<JournalCommand, u64>],
    ids: &[ConversationId],
) {
    let mut db = projector.raw.lock().unwrap();
    for id in ids {
        let expected: Vec<_> = records
            .iter()
            .filter(|record| record.command.conversation_id == *id.as_uuid())
            .flat_map(|record| record.command.events.iter().cloned())
            .collect();
        assert_eq!(
            storage::read_history::<AgentEvent>(&mut db, HistoryQuery::new(*id, 0, 500).unwrap())
                .unwrap()
                .items,
            expected
        );
    }
}
pub(super) fn start(id: ConversationId, seq: u64, turn: Uuid, item: Uuid) -> AgentEvent {
    event(
        id,
        seq,
        EventPayload::ItemStarted {
            turn_id: turn,
            item_id: item,
            kind: ItemKind::AssistantMessage,
        },
    )
}
pub(super) fn delta(
    id: ConversationId,
    seq: u64,
    turn: Uuid,
    item: Uuid,
    text: &str,
) -> AgentEvent {
    event(
        id,
        seq,
        EventPayload::TextDelta {
            turn_id: turn,
            item_id: item,
            text: text.into(),
        },
    )
}

#[test]
fn interleaved_keys_fold_twice_in_first_seen_order_with_exact_scopes_and_ordinals() {
    for (same_conversation, same_item) in [(false, false), (true, false), (false, true)] {
        let (projector, epoch) = fixture();
        let a = ConversationId::from_uuid(Uuid::new_v4());
        let b = if same_conversation {
            a
        } else {
            ConversationId::from_uuid(Uuid::new_v4())
        };
        let ta = Uuid::new_v4();
        let tb = ta;
        let ia = Uuid::new_v4();
        let ib = if same_item { ia } else { Uuid::new_v4() };
        let second = if same_conversation { 2 } else { 1 };
        let a1 = if same_conversation { 3 } else { 2 };
        let b1 = if same_conversation { 4 } else { 2 };
        let a2 = if same_conversation { 5 } else { 3 };
        let records = [
            accepted(
                epoch,
                1,
                command(a, 0, vec![start(a, 1, ta, ia)], Some("A")),
            ),
            accepted(
                epoch,
                2,
                command(
                    b,
                    second - 1,
                    vec![start(b, second, tb, ib)],
                    if same_conversation { None } else { Some("B") },
                ),
            ),
            accepted(
                epoch,
                3,
                command(a, a1 - 1, vec![delta(a, a1, ta, ia, "你\0")], None),
            ),
            accepted(
                epoch,
                4,
                command(b, b1 - 1, vec![delta(b, b1, tb, ib, "β")], None),
            ),
            accepted(
                epoch,
                5,
                command(a, a2 - 1, vec![delta(a, a2, ta, ia, "🙂e\u{301}")], None),
            ),
        ];
        count_bodies(&projector.raw.lock().unwrap());
        projector.apply_batch(&records).unwrap();
        {
            let db = projector.raw.lock().unwrap();
            assert_eq!(
                message(&db, a, ia),
                ("你\0🙂e\u{301}".into(), a2 - 1, false)
            );
            assert_eq!(message(&db, b, ib), ("β".into(), b1 - 1, false));
            let rows: Vec<(String, String, String, u64)> = db
                .prepare("select conversation,item,body,last_ordinal from body_updates order by n")
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(
                rows,
                vec![
                    (
                        a.as_uuid().to_string(),
                        ia.to_string(),
                        "你\0🙂e\u{301}".into(),
                        a2 - 1
                    ),
                    (b.as_uuid().to_string(), ib.to_string(), "β".into(), b1 - 1)
                ]
            );
            assert_eq!(
                db.query_row(
                    "select materialized_position from data_runtime_state",
                    [],
                    |row| row.get::<_, u64>(0)
                )
                .unwrap(),
                5
            );
        }
        let ids = if same_conversation {
            vec![a]
        } else {
            vec![a, b]
        };
        assert_history(&projector, &records, &ids);
        for record in &records {
            assert_eq!(
                projector
                    .lookup(record.operation_id)
                    .unwrap()
                    .unwrap()
                    .result,
                record.result
            );
        }
    }
}

#[test]
fn non_delta_flushes_every_key_before_completion_and_tail_restarts_the_fold() {
    let (projector, epoch) = fixture();
    let a = ConversationId::from_uuid(Uuid::new_v4());
    let b = ConversationId::from_uuid(Uuid::new_v4());
    let ta = Uuid::new_v4();
    let tb = Uuid::new_v4();
    let ia = Uuid::new_v4();
    let ib = Uuid::new_v4();
    let records = [
        accepted(
            epoch,
            1,
            command(a, 0, vec![start(a, 1, ta, ia)], Some("A")),
        ),
        accepted(
            epoch,
            2,
            command(b, 0, vec![start(b, 1, tb, ib)], Some("B")),
        ),
        accepted(
            epoch,
            3,
            command(a, 1, vec![delta(a, 2, ta, ia, "A1")], None),
        ),
        accepted(
            epoch,
            4,
            command(b, 1, vec![delta(b, 2, tb, ib, "B1")], None),
        ),
        accepted(
            epoch,
            5,
            command(a, 2, vec![delta(a, 3, ta, ia, "A2")], None),
        ),
        accepted(
            epoch,
            6,
            command(
                a,
                3,
                vec![event(
                    a,
                    4,
                    EventPayload::ItemCompleted {
                        turn_id: ta,
                        item_id: ia,
                    },
                )],
                None,
            ),
        ),
        accepted(
            epoch,
            7,
            command(b, 2, vec![delta(b, 3, tb, ib, "B2")], None),
        ),
    ];
    {
        let db = projector.raw.lock().unwrap();
        count_bodies(&db);
        db.execute_batch(&format!("create trigger require_all_bodies_before_completion before insert on agent_journal_records
            when json_extract(new.payload_json,'$.payload.type')='itemCompleted' begin
            select case when not exists(select 1 from agent_messages where conversation_id='{}' and item_id='{}' and body='B1' and last_ordinal=1)
            then raise(abort,'other key was not flushed before barrier') end; end;",b.as_uuid(),ib)).unwrap();
    }
    projector.apply_batch(&records).unwrap();
    {
        let db = projector.raw.lock().unwrap();
        assert_eq!(message(&db, a, ia), ("A1A2".into(), 3, true));
        assert_eq!(message(&db, b, ib), ("B1B2".into(), 2, false));
        assert_eq!(
            db.query_row("select count(*) from body_updates", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert_eq!(db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='storedTextSlice'",[],|row|row.get::<_,u64>(0)).unwrap(),2);
    }
    assert_history(&projector, &records, &[a, b]);
}

#[test]
fn reasoning_summary_indices_share_the_arrival_draft_before_canonical_unicode_acceptance() {
    use serde_json::json;
    let (projector, epoch) = fixture();
    let a = ConversationId::from_uuid(Uuid::new_v4());
    let b = ConversationId::from_uuid(Uuid::new_v4());
    let ta = Uuid::new_v4();
    let tb = Uuid::new_v4();
    let ia = Uuid::new_v4();
    let ib = Uuid::new_v4();
    let summary = |value| serde_json::from_value::<EventPayload>(value).unwrap();
    let initial =
        serde_json::from_value::<ItemKind>(json!({"type":"reasoningSummary","parts":[]})).unwrap();
    let payloads=[
        (a,0,vec![event(a,1,EventPayload::ItemStarted{turn_id:ta,item_id:ia,kind:initial}),
            event(a,2,summary(json!({"type":"reasoningSummaryPartAdded","turnId":ta,"itemId":ia,"summaryIndex":"0"}))),
            event(a,3,summary(json!({"type":"reasoningSummaryPartAdded","turnId":ta,"itemId":ia,"summaryIndex":"9223372036854775807"})))],Some("reasoning")),
        (b,0,vec![start(b,1,tb,ib)],Some("B")),
        (a,3,vec![event(a,4,summary(json!({"type":"reasoningSummaryDelta","turnId":ta,"itemId":ia,"summaryIndex":"0","text":"你\0"})))],None),
        (b,1,vec![delta(b,2,tb,ib,"other β")],None),
        (a,4,vec![event(a,5,summary(json!({"type":"reasoningSummaryDelta","turnId":ta,"itemId":ia,"summaryIndex":"9223372036854775807","text":"tail"}))),
            event(a,6,summary(json!({"type":"reasoningSummaryDelta","turnId":ta,"itemId":ia,"summaryIndex":"0","text":"🙂"})))],None),
        (a,6,vec![event(a,7,EventPayload::ItemAccepted{turn_id:ta,item_id:ia,text:"你\0🙂tail".into(),phase:None,source:None,
            kind:serde_json::from_value(json!({"type":"reasoningSummary","parts":[{"index":"0","startByte":0,"lenByte":8},
                {"index":"9223372036854775807","startByte":8,"lenByte":4}]})).unwrap()})],None),
    ];
    let records: Vec<_> = payloads
        .into_iter()
        .enumerate()
        .map(|(i, (id, after, events, title))| {
            accepted(epoch, i as u64 + 1, command(id, after, events, title))
        })
        .collect();
    count_bodies(&projector.raw.lock().unwrap());
    projector.apply_batch(&records).unwrap();
    {
        let db = projector.raw.lock().unwrap();
        let drafts: Vec<String> = db
            .prepare("select body from body_updates order by n")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(drafts, vec!["你\0tail🙂", "other β", "你\0🙂tail"]);
        assert_eq!(message(&db, a, ia), ("你\0🙂tail".into(), 6, true));
        assert_eq!(message(&db, b, ib), ("other β".into(), 1, false));
        assert_eq!(db.query_row("select count(*) from agent_journal_records where json_extract(payload_json,'$.payload.type')='storedReasoningSlice'",[],|row|row.get::<_,u64>(0)).unwrap(),3);
        let parts:Vec<(String,u64,u64)>=db.prepare("select summary_index,start_byte,len_byte from agent_reasoning_parts where conversation_id=?1 and item_id=?2 order by part_ordinal").unwrap()
            .query_map(params![a.as_uuid().to_string(),ia.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(
            parts,
            vec![("0".into(), 0, 8), ("9223372036854775807".into(), 8, 4)]
        );
    }
    assert_history(&projector, &records, &[a, b]);
}
