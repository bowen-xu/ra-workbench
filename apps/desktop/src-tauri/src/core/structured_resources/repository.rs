use super::{Draft, Error, ReadResult, Resource, TemplateRef, content, decode, encode};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

pub(crate) fn ensure_paper_resources(
    db: &Connection,
    paper_id: &str,
) -> Result<Vec<Resource>, Error> {
    let exists: bool = db.query_row(
        "select exists(select 1 from bibliographic_items where id=?1 and trashed_at is null)",
        [paper_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(Error::NotFound);
    }
    for (kind, title) in [
        ("portrait", "文献画像"),
        ("structuredReading", "结构化阅读"),
    ] {
        db.execute("insert into structured_resources(id,kind,paper_id,title) values(?1,?2,?3,?4) on conflict(paper_id,kind) do nothing", params![uuid::Uuid::new_v4().to_string(),kind,paper_id,title])?;
    }
    list_headers(db, Some(paper_id))
}

pub(crate) fn find_by_paper(db: &Connection, paper: &str, kind: &str) -> Result<Resource, Error> {
    let id: String = db
        .query_row(
            "select r.id from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id where coalesce(r.paper_id,o.paper_id)=?1 and r.kind=?2",
            params![paper, kind],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    get(db, &id)
}

pub(crate) fn list(db: &Connection, paper: Option<&str>) -> Result<Vec<Resource>, Error> {
    let mut stmt = db.prepare("select r.id from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id left join bibliographic_items p on p.id=coalesce(r.paper_id,o.paper_id) where (?1 is null or coalesce(r.paper_id,o.paper_id)=?1) and (coalesce(r.paper_id,o.paper_id) is null or p.trashed_at is null) order by r.created_at,r.id")?;
    let ids = stmt
        .query_map([paper], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| get(db, id)).collect()
}

pub(crate) fn get_header(db: &Connection, id: &str) -> Result<Resource, Error> {
    let row=db.query_row("select r.id,r.kind,coalesce(r.paper_id,o.paper_id),r.title,r.revision from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id left join bibliographic_items p on p.id=coalesce(r.paper_id,o.paper_id) where r.id=?1 and (coalesce(r.paper_id,o.paper_id) is null or p.trashed_at is null)",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?))).optional()?.ok_or(Error::NotFound)?;
    Ok(Resource {
        source_uri: resource_path(row.2.as_deref(), &row.0),
        id: row.0,
        kind: row.1,
        paper_id: row.2,
        title: row.3,
        revision: row.4,
        content: None,
        review_baseline: None,
        pending_field_ids: vec![],
        reviewed_field_ids: vec![],
        template_ref: None,
        draft: None,
    })
}
pub(crate) fn has_owned_draft(
    db: &Connection,
    id: &str,
    conversation: &str,
) -> Result<bool, Error> {
    Ok(db.query_row("select exists(select 1 from structured_resource_drafts where resource_id=?1 and creator_conversation_id=?2) or exists(select 1 from structured_resource_creations where resource_id=?1 and creator_conversation_id=?2)",params![id,conversation],|r|r.get(0))?)
}
pub(crate) fn get_version(db: &Connection, id: &str, revision: i64) -> Result<Resource, Error> {
    let mut resource = get_header(db, id)?;
    let (raw,baseline,reviewed,template_id,template_revision)=db.query_row("select content_json,baseline_json,reviewed_fields_json,template_id,template_revision from structured_resource_revisions where resource_id=?1 and revision=?2",params![id,revision],|r|Ok((r.get::<_,Option<String>>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?,r.get::<_,Option<i64>>(4)?))).optional()?.ok_or(Error::NotFound)?;
    resource.revision = revision;
    resource.content = raw.as_deref().map(decode).transpose()?;
    resource.review_baseline = baseline.as_deref().map(decode).transpose()?;
    resource.reviewed_field_ids = decode(&reviewed)?;
    resource.pending_field_ids =
        content::pending(resource.content.as_ref(), resource.review_baseline.as_ref());
    resource.template_ref = template_id
        .zip(template_revision)
        .map(|(resource_id, revision)| TemplateRef {
            resource_id,
            revision,
        });
    Ok(resource)
}
pub(crate) fn list_headers(db: &Connection, paper: Option<&str>) -> Result<Vec<Resource>, Error> {
    let mut stmt=db.prepare("select r.id from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id left join bibliographic_items p on p.id=coalesce(r.paper_id,o.paper_id) where (?1 is null or coalesce(r.paper_id,o.paper_id)=?1) and (coalesce(r.paper_id,o.paper_id) is null or p.trashed_at is null) order by r.created_at,r.id")?;
    let ids = stmt
        .query_map([paper], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter().map(|id| get_header(db, id)).collect()
}

pub(crate) fn get(db: &Connection, id: &str) -> Result<Resource, Error> {
    let row = db.query_row("select r.id,r.kind,coalesce(r.paper_id,o.paper_id),r.title,r.revision,r.content_json,r.baseline_json,r.reviewed_fields_json,r.template_id,r.template_revision from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id left join bibliographic_items p on p.id=coalesce(r.paper_id,o.paper_id) where r.id=?1 and (coalesce(r.paper_id,o.paper_id) is null or p.trashed_at is null)", [id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?,r.get::<_,String>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?,r.get::<_,Option<String>>(8)?,r.get::<_,Option<i64>>(9)?))).optional()?.ok_or(Error::NotFound)?;
    let content: Option<Value> = row.5.as_deref().map(decode).transpose()?;
    let baseline = row.6.as_deref().map(decode).transpose()?;
    let source_uri = resource_path(row.2.as_deref(), &row.0);
    Ok(Resource {
        pending_field_ids: content::pending(content.as_ref(), baseline.as_ref()),
        id: row.0,
        kind: row.1,
        paper_id: row.2,
        source_uri,
        title: row.3,
        revision: row.4,
        content,
        review_baseline: baseline,
        reviewed_field_ids: decode(&row.7)?,
        template_ref: row.8.zip(row.9).map(|(resource_id, revision)| TemplateRef {
            resource_id,
            revision,
        }),
        draft: None,
    })
}

pub(super) fn get_draft(
    db: &Connection,
    id: &str,
    conversation: Option<&str>,
) -> Result<Option<Draft>, Error> {
    let scope = conversation.unwrap_or("human");
    db.query_row("select id,resource_id,base_revision,revision,content,content_hash,creator_conversation_id,errors_json from structured_resource_drafts where resource_id=?1 and creator_scope=?2",params![id,scope], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,Option<String>>(6)?,r.get::<_,String>(7)?))).optional()?.map(|r| Ok(Draft{id:r.0,resource_id:r.1,base_revision:r.2,revision:r.3,content:r.4,content_hash:r.5,creator_conversation_id:r.6,errors:decode(&r.7)?})).transpose()
}

pub(crate) fn read_text(
    db: &Connection,
    id: &str,
    conversation: Option<&str>,
    saved: bool,
) -> Result<ReadResult, Error> {
    let mut resource = get(db, id)?;
    let draft = if saved {
        None
    } else {
        get_draft(db, id, conversation)?
    };
    let text = draft
        .as_ref()
        .map(|d| d.content.clone())
        .unwrap_or_else(|| {
            resource
                .content
                .as_ref()
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .unwrap_or_default()
        });
    resource.draft = draft.clone();
    Ok(ReadResult {
        resource,
        text,
        draft,
    })
}

pub(super) fn persist_revision(
    db: &Connection,
    resource: &Resource,
    editor: &str,
    source: Option<&str>,
    protected: &[String],
) -> Result<(), Error> {
    let requested_source = source;
    let content = resource.content.as_ref().map(encode).transpose()?;
    let baseline = resource.review_baseline.as_ref().map(encode).transpose()?;
    let reviewed = encode(&resource.reviewed_field_ids)?;
    let protected = encode(&protected)?;
    let inherited: Option<String> = if source.is_none() {
        db.query_row("select source_snapshot_id from structured_resource_revisions where resource_id=?1 and source_snapshot_id is not null order by revision desc limit 1",[&resource.id],|r|r.get(0)).optional()?.flatten()
    } else {
        None
    };
    let source = source.or(inherited.as_deref());
    db.execute("update structured_resources set revision=?2,content_json=?3,baseline_json=?4,reviewed_fields_json=?5,protected_fields_json=?6,template_id=?7,template_revision=?8,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') where id=?1",params![resource.id,resource.revision,content,baseline,reviewed,protected,resource.template_ref.as_ref().map(|r|&r.resource_id),resource.template_ref.as_ref().map(|r|r.revision)])?;
    db.execute("insert into structured_resource_revisions(resource_id,revision,content_json,baseline_json,reviewed_fields_json,protected_fields_json,editor_kind,source_snapshot_id,template_id,template_revision) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![resource.id,resource.revision,content,baseline,reviewed,protected,editor,source,resource.template_ref.as_ref().map(|r|&r.resource_id),resource.template_ref.as_ref().map(|r|r.revision)])?;
    super::provenance::persist(db, resource, requested_source, source)?;
    Ok(())
}

fn resource_path(paper: Option<&str>, id: &str) -> String {
    match paper {
        Some(paper) => research_agent_core::text_patch::address::object(
            research_agent_core::text_patch::address::SourceRoot::Library,
            &[paper.into(), id.into()],
        ),
        None => research_agent_core::text_patch::address::object(
            research_agent_core::text_patch::address::SourceRoot::Agents,
            &[
                "skills".into(),
                "edit-structured-resources".into(),
                "assets".into(),
                "reading".into(),
                format!("{id}.json"),
            ],
        ),
    }
}
