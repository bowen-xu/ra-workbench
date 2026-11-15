use super::fixture::{self, Fixture};
use crate::agent::{
    context_session::{self, queue, sources::Resource},
    queue_contract::Confirmation,
    repository::{input_queue, submissions},
};
use research_agent_core::input_queue::{QueueEdit, QueuedInput};
use research_agent_core::user_input::UserInput;
use uuid::Uuid;

#[tokio::test]
async fn queued_context_resolution_preserves_raw_identity_and_atomic_preview() {
    let fixture = Fixture::new();
    let note = Resource::Note {
        resource_id: fixture.note,
    };
    let mentions = vec![fixture::mention(&note)];
    let input_id = Uuid::new_v4();
    let request_id = Uuid::new_v4();
    let request = || {
        let mut request = fixture.request(
            vec![fixture.group(true, false), note.clone()],
            mentions.clone(),
        );
        request.request_id = request_id;
        request.queue = Some(Confirmation {
            input_id,
            expected_revision: 1,
        });
        request
    };
    let raw = request().user_input;
    {
        let mut db = fixture.store.connection().unwrap();
        input_queue::apply_checked(
            &mut db,
            fixture.conversation,
            Uuid::new_v4(),
            0,
            QueueEdit::Add(QueuedInput {
                id: input_id,
                text: request().text,
                user_input: raw.clone(),
                origin: None,
            }),
            |db| queue::authorize(db, fixture.conversation),
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        let preview =
            context_session::materialize_in(&tx, &fixture.store, request(), fixture::model())
                .unwrap();
        assert_ne!(preview.input.user_input, raw);
        assert_eq!(preview.queue.unwrap().input_id, input_id);
        tx.rollback().unwrap();
        let state = input_queue::read_checked(&mut db, fixture.conversation, |db| {
            queue::authorize(db, fixture.conversation)
        })
        .unwrap();
        assert_eq!(state.revision(), 1);
        assert_eq!(state.inputs()[0].user_input, raw);
        assert!(state.preparing().is_none());
        assert!(
            submissions::read(&db, fixture.conversation, request_id)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        queue::pending_queue(
            &fixture.store,
            queue::Scope {
                conversation_id: fixture.conversation
            }
        )
        .await
        .unwrap()
        .is_none()
    );
    let committed = {
        let mut db = fixture.store.connection().unwrap();
        let tx = db.transaction().unwrap();
        let mut changed = request();
        changed.text.push_str(" changed after queue edit");
        let UserInput::Text { text, .. } = &mut changed.user_input[0] else {
            panic!("expected the original composer text item");
        };
        *text = changed.text.clone();
        assert!(matches!(
            context_session::materialize_in(&tx, &fixture.store, changed, fixture::model()),
            Err(context_session::Error::ConfirmationMismatch)
        ));
        tx.rollback().unwrap();
        let tx = db.transaction().unwrap();
        let preview =
            context_session::materialize_in(&tx, &fixture.store, request(), fixture::model())
                .unwrap();
        tx.commit().unwrap();
        let saved = submissions::read(&db, fixture.conversation, request_id)
            .unwrap()
            .unwrap();
        assert_eq!(saved.input, preview.input);
        assert_eq!(saved.turn_id, preview.turn_id);
        let state = input_queue::read_checked(&mut db, fixture.conversation, |db| {
            queue::authorize(db, fixture.conversation)
        })
        .unwrap();
        assert_eq!(state.inputs()[0].user_input, raw);
        assert_eq!(state.preparing(), Some(input_id));
        assert_eq!(state.revision(), 2);
        preview
    };
    let pending = queue::pending_queue(
        &fixture.store,
        queue::Scope {
            conversation_id: fixture.conversation,
        },
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(pending.input, committed.input);
    assert_eq!(pending.turn_id, committed.turn_id);
    assert_eq!(pending.context_snapshot_id, committed.context_snapshot_id);
    assert_eq!(pending.queue.unwrap().expected_revision, 2);
}
