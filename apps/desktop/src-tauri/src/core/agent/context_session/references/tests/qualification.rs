use crate::agent::context_session::{self, resource_grants, sources::Resource};
use research_agent_core::{submission::SubmissionRecord, user_input::UserInput};
use serde_json::Value;

#[path = "qualification_fixture.rs"]
mod fixture;
#[path = "qualification_queue.rs"]
mod queue;
#[path = "qualification_restore.rs"]
mod restore;

#[test]
fn current_mentions_prepare_identity_only_grants_without_domain_sources() {
    let fixture = fixture::Fixture::new();
    let note = Resource::Note {
        resource_id: fixture.note,
    };
    let portrait = Resource::StructuredObject {
        resource_id: fixture.portrait.clone(),
        access: None,
    };
    let mut db = fixture.store.connection().unwrap();
    for dynamic in [false, true] {
        // The current composer adds each context: mention to resources. The
        // selected group already covers this Note, so it must not add a grant.
        let request = fixture.request(
            vec![
                fixture.group(dynamic, false),
                note.clone(),
                portrait.clone(),
            ],
            vec![fixture::mention(&note), fixture::mention(&portrait)],
        );
        let tx = db.transaction().unwrap();
        let preview =
            context_session::materialize_in(&tx, &fixture.store, request, fixture::model())
                .unwrap();
        assert_eq!(preview.input.context.len(), 2);
        for source in &preview.input.context {
            let anchor: Value = serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
            assert_eq!(anchor["content"], "resourceGrant");
            assert!(!source.text.contains(fixture::NOTE_BODY));
            assert!(!source.text.contains(fixture::PORTRAIT_BODY));
        }
        let member = resource_grants::note_reference(&tx, &preview.input.context, fixture.note)
            .unwrap()
            .unwrap();
        let anchor: Value = serde_json::from_str(member.anchor.as_deref().unwrap()).unwrap();
        assert_eq!(anchor["contextAccess"], "readOnly");
        let structured = preview
            .input
            .context
            .iter()
            .find(|source| source.resource_id == fixture.portrait)
            .unwrap();
        for (input, source) in preview
            .input
            .user_input
            .iter()
            .skip(1)
            .zip([&member, structured])
        {
            let expected = context_session::references::frozen_file_binding(source);
            let encoded = serde_json::to_value(input).unwrap();
            assert_eq!(encoded["target"]["sourceId"], expected.0);
            assert_eq!(encoded["target"]["resourceId"], expected.1);
            assert_eq!(encoded["target"]["sourceUrl"], expected.2.unwrap());
        }
        assert_eq!(
            serde_json::to_value(&preview.input.user_input[1]).unwrap()["target"]["sourceUrl"],
            format!("source://notes/{}", fixture.note)
        );
        let record = SubmissionRecord {
            conversation_id: preview.conversation_id,
            request_id: preview.request_id,
            turn_id: preview.turn_id,
            context_snapshot_id: preview.context_snapshot_id,
            input: preview.input,
        };
        assert!(
            context_session::structured::access(&tx, &record, &fixture.portrait, false).unwrap()
        );
        tx.rollback().unwrap();
    }

    // Raw unresolved references alone do not grant access. There is deliberately
    // no explicit Note/StructuredObject resource overriding these exclusions.
    for dynamic in [false, true] {
        let raw: Vec<UserInput> = vec![
            fixture::mention(&note),
            fixture::mention(&Resource::Note {
                resource_id: fixture.foreign_note,
            }),
            fixture::mention(&portrait),
        ];
        let request = fixture.request(vec![fixture.group(dynamic, true)], raw.clone());
        let original_input = request.user_input.clone();
        let tx = db.transaction().unwrap();
        let preview =
            context_session::materialize_in(&tx, &fixture.store, request, fixture::model())
                .unwrap();
        assert_eq!(preview.input.user_input, original_input);
        for id in [fixture.note, fixture.foreign_note] {
            assert!(
                resource_grants::note_reference(&tx, &preview.input.context, id)
                    .unwrap()
                    .is_none()
            );
        }
        let record = SubmissionRecord {
            conversation_id: preview.conversation_id,
            request_id: preview.request_id,
            turn_id: preview.turn_id,
            context_snapshot_id: preview.context_snapshot_id,
            input: preview.input,
        };
        assert!(
            !context_session::structured::access(&tx, &record, &fixture.portrait, false).unwrap()
        );
        tx.rollback().unwrap();
    }
    assert_eq!(
        resource_grants::grants(
            &db,
            &[Resource::Note {
                resource_id: fixture.foreign_note
            }],
            None
        )
        .unwrap()
        .len(),
        1,
        "explicit standalone Note selection remains an independent grant"
    );
}
