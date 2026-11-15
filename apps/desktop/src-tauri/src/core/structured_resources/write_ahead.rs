//! Persist raw text and the request identity before any JSON/domain validation.
use super::drafts::transaction;
use super::{Draft, Error, WriteDraft, encode, get, hash, repository};
use crate::{ai::structured_resource_validation::Flow, hbs_storage::boundary::BoundaryError};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(super) fn stage(
    db: &mut Connection,
    input: &WriteDraft,
    fingerprint: &str,
) -> Result<Draft, Error> {
    stage_with::<std::convert::Infallible>(db, input, fingerprint, false)
        .map_err(BoundaryError::into_static)
}
pub(super) fn stage_with<P>(
    db: &mut Connection,
    input: &WriteDraft,
    fingerprint: &str,
    explicit: bool,
) -> Result<Draft, BoundaryError<P, Error>> {
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 200
        || input.content.len() > 8 * 1024 * 1024
        || !["human", "agent", "migration"].contains(&input.editor_kind.as_str())
    {
        return Err(transaction::before(
            Error::Invalid("写入请求身份、来源或容量无效".into()),
            "execution",
        ));
    }

    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| transaction::before(error, "begin"))?;
    let mut phase = "read";
    let result = (|| -> Result<(Draft, bool), Flow<Error, P>> {
        let resource = get(&tx, &input.resource_id)?;
        let old = repository::get_draft(&tx, &resource.id, input.conversation_id.as_deref())?;
        if let Some((stored,id,revision))=tx.query_row("select request_hash,draft_id,draft_revision from structured_resource_write_intents where request_id=?1",[&input.request_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?))).optional()?{
        if stored!=fingerprint{return Err(Error::Invalid("请求身份已用于不同内容".into()).into());}
        let draft=old.ok_or(Error::Conflict)?;
        if draft.id!=id||draft.revision!=revision{return Err(Error::Conflict.into());}
        return Ok((draft,false));
    }
        if input
            .expected_draft_revision
            .is_some_and(|revision| old.as_ref().map(|d| d.revision) != Some(revision))
        {
            return Err(Error::Conflict.into());
        }
        let pending_source:Option<String>=tx.query_row("select source_snapshot_id from structured_resource_drafts where resource_id=?1 and creator_scope=?2",params![resource.id,input.conversation_id.as_deref().unwrap_or("human")],|r|r.get::<_,Option<String>>(0)).optional()?.flatten();
        let unchanged = old.as_ref().is_some_and(|d| {
            d.content == input.content
                && d.base_revision == input.expected_revision
                && input
                    .source_snapshot_id
                    .as_ref()
                    .is_none_or(|source| Some(source) == pending_source.as_ref())
        });
        let draft = Draft {
            id: old
                .as_ref()
                .map(|d| d.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            resource_id: resource.id.clone(),
            base_revision: input.expected_revision,
            revision: old.as_ref().map_or(1, |d| {
                if unchanged {
                    d.revision
                } else {
                    d.revision + 1
                }
            }),
            content: input.content.clone(),
            content_hash: hash(&input.content),
            creator_conversation_id: input.conversation_id.clone(),
            errors: vec![],
        };
        phase = "write";
        tx.execute("insert into structured_resource_drafts(id,resource_id,creator_scope,creator_conversation_id,base_revision,revision,content,content_hash,source_snapshot_id,errors_json) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) on conflict(resource_id,creator_scope) do update set base_revision=excluded.base_revision,revision=excluded.revision,content=excluded.content,content_hash=excluded.content_hash,source_snapshot_id=excluded.source_snapshot_id,errors_json=excluded.errors_json,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",params![draft.id,resource.id,input.conversation_id.as_deref().unwrap_or("human"),input.conversation_id,draft.base_revision,draft.revision,draft.content,draft.content_hash,input.source_snapshot_id.as_ref().or(pending_source.as_ref()),encode(&draft.errors)?])?;

        tx.execute("insert into structured_resource_write_intents(request_id,resource_id,request_hash,draft_id,draft_revision,creator_scope) values(?1,?2,?3,?4,?5,?6)",params![input.request_id,resource.id,fingerprint,draft.id,draft.revision,input.conversation_id.as_deref().unwrap_or("human")])?;
        Ok((draft, true))
    })();
    let settled = transaction::settle(tx, result, phase, explicit);
    transaction::confirm(db, settled, explicit)
}
