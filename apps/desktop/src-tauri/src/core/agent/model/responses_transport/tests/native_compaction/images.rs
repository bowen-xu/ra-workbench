use super::*;
use research_agent_core::{
    context::checkpoint::{StoredContent, StoredMessage},
    submission::{ImageMaterializer, SubmissionFuture, SubmissionImage},
};
use sha2::{Digest, Sha256};

pub(super) struct FrozenImages {
    pub conversation: Uuid,
    pub request: Uuid,
    pub turn: Uuid,
}
fn image() -> SubmissionImage {
    let bytes = b"\x89PNG\r\n\x1a\nfrozen synthetic image".to_vec();
    SubmissionImage {
        source_id: "source-one".into(),
        media_type: "image/png".into(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        bytes,
    }
}
impl ImageMaterializer for FrozenImages {
    fn materialize(
        &self,
        conversation: Uuid,
        request: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        Box::pin(async move {
            assert_eq!(conversation, self.conversation);
            Ok(if request == self.request {
                vec![image()]
            } else {
                vec![]
            })
        })
    }
    fn materialize_history(
        &self,
        conversation: Uuid,
        turn: Uuid,
    ) -> SubmissionFuture<'_, Vec<SubmissionImage>> {
        Box::pin(async move {
            assert_eq!(conversation, self.conversation);
            Ok(if turn == self.turn {
                vec![image()]
            } else {
                vec![]
            })
        })
    }
}

async fn run(
    fixture: &Fixture,
    record: &SubmissionRecord,
    url: &str,
    images: Arc<FrozenImages>,
    compact: bool,
) {
    let execution = Arc::new(fixture.execution(record, fixture.transport(record, url)));
    let admission = if compact {
        execution
            .start_compaction(
                fixture.service.clone(),
                fixture.store.clone(),
                ConversationId::from_uuid(fixture.id),
                record.request_id,
                Some(images),
            )
            .await
    } else {
        execution
            .start_with_images(
                fixture.service.clone(),
                fixture.store.clone(),
                images,
                ConversationId::from_uuid(fixture.id),
                record.request_id,
            )
            .await
    }
    .unwrap();
    let SubmissionAdmission::Started(owned) = admission else {
        panic!("new task expected")
    };
    let completion = owned.task.wait().await;
    if completion != TaskCompletion::Completed {
        let db = fixture.app.connection().unwrap();
        let failures: Vec<String> = db
            .prepare(
                "select payload_json from agent_items where conversation_id=?1 order by ordinal",
            )
            .unwrap()
            .query_map([fixture.id.to_string()], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .filter(|row| row.contains("modelRequestFailed"))
            .collect();
        panic!(
            "input={:?}, compact={compact}, completion={completion:?}, failures={failures:?}",
            record.input.text
        );
    }
}

fn check_image_user(request: &Value, text: &str) {
    let input = request["input"].as_array().unwrap();
    let image_users: Vec<_> = input
        .iter()
        .filter(|item| {
            item["content"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|part| part["type"] == "input_image"))
        })
        .collect();
    assert_eq!(image_users.len(), 1, "one original user owns the image");
    assert_eq!(image_users[0]["role"], "user");
    let parts = image_users[0]["content"].as_array().unwrap();
    assert_eq!(parts.len(), if text.is_empty() { 1 } else { 2 });
    assert_eq!(parts[0]["type"], "input_image");
    if !text.is_empty() {
        assert_eq!(parts[1]["text"], text);
    }
}

#[tokio::test]
async fn native_v2_images_stay_with_original_user_across_two_compactions() {
    exercise(false).await;
}

#[tokio::test]
async fn native_v2_pure_image_submission_survives_two_compactions_without_placeholder_text() {
    exercise(true).await;
}

async fn exercise(only_images: bool) {
    let fixture = Fixture::new().await;
    let replacement = json!({"type":"compaction","encrypted_content":"replacement image state"});
    let (url, peer) = sequential_server(vec![
        ok(&body()),
        ok(&events(&[opaque()], true)),
        ok(&body()),
        ok(&events(std::slice::from_ref(&replacement), true)),
        ok(&body()),
    ])
    .await;
    let first = if only_images {
        let mut input = fixture.record("template").await.input;
        input.text.clear();
        fixture
            .store
            .put_with_images(fixture.id, Uuid::new_v4(), input, vec![image()])
            .await
            .unwrap()
    } else {
        fixture.record("original image question").await
    };
    let images = Arc::new(FrozenImages {
        conversation: fixture.id,
        request: first.request_id,
        turn: first.turn_id,
    });
    run(&fixture, &first, &url, images.clone(), false).await;
    for (text, compact) in [
        ("/compact", true),
        ("continue", false),
        ("/compact", true),
        ("after second", false),
    ] {
        let record = fixture.record(text).await;
        run(&fixture, &record, &url, images.clone(), compact).await;
        if compact {
            let checkpoint = fixture
                .service
                .active_context_checkpoint(fixture.id)
                .await
                .unwrap()
                .unwrap();
            let StoredMessage::User { content, .. } = &checkpoint.retained[0] else {
                panic!("user expected")
            };
            assert!(
                matches!(&content[0], StoredContent::Image { sha256, .. } if sha256 == &image().sha256)
            );
            if only_images {
                assert_eq!(content.len(), 1);
            } else {
                assert!(
                    matches!(&content[..], [StoredContent::Image { .. }, StoredContent::Text { text }] if text == "original image question")
                );
            }
            assert!(checkpoint.canonical_context.as_ref().is_none_or(|canonical| !canonical.messages.iter().any(|message| matches!(message, StoredMessage::User {
source_item_id: _, content } if content.iter().any(|part| matches!(part, StoredContent::Image { .. }))))));
        }
    }
    let requests = peer.await.unwrap();
    assert_eq!(requests.len(), 5);
    for request in &requests {
        check_image_user(request, &first.input.text);
    }
    assert_eq!(
        requests[4]["input"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| **item == replacement)
            .count(),
        1
    );
    assert!(!requests[4]["input"].as_array().unwrap().contains(&opaque()));
    let history = fixture
        .service
        .open(ConversationId::from_uuid(fixture.id))
        .await
        .unwrap();
    assert!(
        history
            .turn(first.turn_id)
            .unwrap()
            .items
            .values()
            .any(|item| item.text == first.input.text && item.completed)
    );
}

#[tokio::test]
async fn pure_image_receipt_requires_exact_valid_manifest_and_retry_identity() {
    let fixture = Fixture::new().await;
    let mut input = fixture.record("template").await.input;
    input.text.clear();
    let request = Uuid::new_v4();
    assert!(
        fixture
            .store
            .put(fixture.id, request, input.clone())
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .put_with_images(fixture.id, request, input.clone(), vec![])
            .await
            .is_err()
    );
    let first = fixture
        .store
        .put_with_images(fixture.id, request, input.clone(), vec![image()])
        .await
        .unwrap();
    let retry = fixture
        .store
        .put_with_images(fixture.id, request, input.clone(), vec![image()])
        .await
        .unwrap();
    assert_eq!(first, retry);
    assert!(
        fixture
            .store
            .get_image_references(Uuid::new_v4(), request)
            .await
            .is_err()
    );
    let mut changed = image();
    changed.source_id = "different source".into();
    assert!(
        fixture
            .store
            .put_with_images(fixture.id, request, input.clone(), vec![changed])
            .await
            .is_err()
    );
    assert!(
        fixture
            .store
            .put_with_images(fixture.id, request, input, vec![])
            .await
            .is_err()
    );
    assert_eq!(fixture.store.get(fixture.id, request).await.unwrap(), first);
}
