//! Original failed-publication read-back and retained-draft recovery.
use super::*;
pub(super) fn recover<P>(
    db: &mut Connection,
    input: WriteDraft,
    fingerprint: &str,
    staged: &super::super::Draft,
    error: rusqlite::Error,
    explicit: bool,
) -> Result<WriteResult, BoundaryError<P, Error>> {
    if let Some(result) = query_outcome(db, &input.request_id)
        .map_err(|error| transaction::before(error, "receipt"))?
    {
        return Ok(result);
    }
    let mut draft = repository::get_draft(db, &input.resource_id, input.conversation_id.as_deref())
        .map_err(|error| transaction::before(error, "read"))?
        .ok_or_else(|| transaction::before(Error::Database(error), "read"))?;
    if draft.id != staged.id || draft.revision != staged.revision {
        return Err(transaction::before(Error::Conflict, "read"));
    }
    draft.errors = vec![ValidationIssue {
        path: "$".into(),
        code: "storage_error".into(),
        message: "正式保存失败；已提交的草稿保留，可重试写入".into(),
    }];
    let tx = db
        .transaction()
        .map_err(|error| transaction::before(error, "begin"))?;
    let result = (|| -> Result<(WriteResult, bool), Flow<Error, P>> {
        tx.execute(
            "update structured_resource_drafts set errors_json=?3 where id=?1 and revision=?2",
            params![draft.id, draft.revision, encode(&draft.errors)?],
        )?;
        let mut resource = get(&tx, &input.resource_id)?;
        resource.draft = Some(draft.clone());
        let result = WriteResult {
            status: "invalid".into(),
            resource,
            draft: Some(draft.clone()),
            errors: draft.errors,
            receipt: None,
            validation_origin: None,
        };
        save_operation(&tx, &input, fingerprint, &result)?;
        Ok((result, true))
    })();
    let settled = transaction::settle(tx, result, "write", explicit);
    transaction::confirm(db, settled, explicit)
}
