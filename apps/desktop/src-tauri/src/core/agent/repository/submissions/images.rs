//! Atomic admission of validated frozen image metadata beside the original input.
//! The caller's content adapter owns bytes and resource permission, as before.
use super::*;

impl SqliteSubmissionStore {
    #[cfg(test)]
    pub async fn put_with_images(
        &self,
        conversation_id: Uuid,
        request_id: Uuid,
        input: SubmissionInput,
        images: Vec<SubmissionImage>,
    ) -> Result<SubmissionRecord, SubmissionError> {
        self.run(move |db| {
            let tx = db
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(db_error)?;
            let record = insert_in_with_images(&tx, conversation_id, request_id, input, &images)?;
            tx.commit().map_err(db_error)?;
            Ok(record)
        })
        .await
    }
}

pub(crate) fn insert_in_with_images(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: Uuid,
    request_id: Uuid,
    input: SubmissionInput,
    images: &[SubmissionImage],
) -> Result<SubmissionRecord, SubmissionError> {
    sql::ensure_no_child_cleanup(tx, conversation_id)
        .map_err(|error| error.domain_or(SubmissionError::Unavailable))?;
    validate_submission_images(images)?;
    let references = images
        .iter()
        .enumerate()
        .map(|(ordinal, image)| {
            if format!("{:x}", Sha256::digest(&image.bytes)) != image.sha256 {
                return Err(SubmissionError::InvalidInput);
            }
            Ok(SubmissionImageReference {
                source_id: image.source_id.clone(),
                content_sha256: image.sha256.clone(),
                media_type: image.media_type.clone(),
                byte_size: image.bytes.len() as u64,
                ordinal: ordinal as u32,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    input.validate_with_image_references(&references)?;
    if let Some(record) = read(tx, conversation_id, request_id)? {
        return if record.input == input
            && read_image_manifest(tx, conversation_id, request_id)? == references
        {
            Ok(record)
        } else {
            Err(SubmissionError::Conflict)
        };
    }
    let record = insert_validated(tx, conversation_id, request_id, input)?;
    insert_image_manifest(tx, conversation_id, request_id, images)?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_preserves_pure_image_receipt_and_missing_manifest_blocks_restore() {
        let mut db = Connection::open_in_memory().unwrap();
        crate::storage::migrate(&mut db).unwrap();
        let conversation = Uuid::new_v4();
        crate::agent::repository::create_conversation(
            &mut db,
            research_agent_core::conversation::ConversationId::from_uuid(conversation),
            "image fixture",
        )
        .unwrap();
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic image".to_vec();
        let image = SubmissionImage {
            source_id: "frozen-image".into(),
            media_type: "image/png".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes,
        };
        let input = SubmissionInput {
            remote_compaction_v2: None,
            text: String::new(),
            user_input: vec![],
            skill_instructions: vec![],
            model: research_agent_core::protocol::ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "vision".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context: vec![],
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        };
        let request = Uuid::new_v4();
        let retry_request = Uuid::new_v4();
        let tx = db.transaction().unwrap();
        let source = insert_in_with_images(&tx, conversation, request, input, &[image]).unwrap();
        let retry = retry_frozen_in(&tx, conversation, request, retry_request).unwrap();
        assert_eq!(retry.input, source.input);
        assert_ne!(retry.turn_id, source.turn_id);
        assert_eq!(
            read_image_manifest(&tx, conversation, request).unwrap(),
            read_image_manifest(&tx, conversation, retry_request).unwrap()
        );
        assert_eq!(read(&tx, conversation, retry_request).unwrap(), Some(retry));
        tx.execute(
            "delete from agent_submission_images where conversation_id=?1 and request_id=?2",
            params![conversation.to_string(), retry_request.to_string()],
        )
        .unwrap();
        assert!(matches!(
            read(&tx, conversation, retry_request),
            Err(SubmissionError::Corrupt)
        ));
        assert_eq!(read(&tx, conversation, request).unwrap(), Some(source));
        tx.commit().unwrap();
    }
}
