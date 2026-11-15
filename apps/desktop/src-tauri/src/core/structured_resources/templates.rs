use super::{
    AdoptionDraft, Error, Resource, TemplateRef, decode, drafts, encode, find_by_paper, get,
    get_header, get_version, hash, repository,
};
use crate::{
    ai::structured_resource_validation::StructuredPolicy,
    hbs_storage::boundary::{self, BoundaryError, HbsSources, StaticHbsSources},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

pub(crate) fn create_template(
    db: &Connection,
    title: &str,
    request: &str,
    conversation: Option<&str>,
) -> Result<Resource, Error> {
    if title.trim().is_empty()
        || title.chars().count() > 120
        || request.trim().is_empty()
        || request.len() > 200
    {
        return Err(Error::Invalid("模板名称或创建请求无效".into()));
    }
    if let Some((id,owner))=db.query_row("select resource_id,creator_conversation_id from structured_resource_creations where request_id=?1",[request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional()?{
        if owner.as_deref()!=conversation{return Err(Error::Invalid("创建请求不属于当前对话".into()));}
        return get_header(db,&id);
    }
    let tx = db.unchecked_transaction()?;
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "insert into structured_resources(id,kind,title) values(?1,'readingTemplate',?2)",
        params![id, title.trim()],
    )?;
    tx.execute("insert into structured_resource_creations(request_id,resource_id,creator_conversation_id) values(?1,?2,?3)",params![request,id,conversation])?;
    tx.commit()?;
    get_header(db, &id)
}

pub(crate) fn ensure_paper_template(db: &Connection, paper: &str) -> Result<Resource, Error> {
    // The migration and creation trigger allocate this identity before generation.
    find_by_paper(db, paper, "readingTemplate")
}

pub(crate) fn adopt_template(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: AdoptionDraft,
) -> Result<Resource, Error> {
    adopt_template_with(
        &mut StaticHbsSources::new(Some(policy), Some(reading_source)),
        db,
        input,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn adopt_template_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    input: AdoptionDraft,
) -> Result<Resource, BoundaryError<<S::Policy as StructuredPolicy>::Pause, Error>> {
    use super::review::boundary::{before_transaction, settle};
    let fingerprint = hash(&encode(&input).map_err(|error| before_transaction(error, "receipt"))?);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| before_transaction(error, "begin"))?;
    let mut phase = "receipt";
    let result = (|| {
        if let Some(receipt) = drafts::receipt(&tx, &input.request_id, &fingerprint)? {
            return Ok((get(&tx, &receipt.resource_id)?, false));
        }
        phase = "read";
        let template = get_version(&tx, &input.template_id, input.template_revision)?;
        if template.kind != "readingTemplate" || template.content.is_none() {
            return Err(Error::Invalid("所选模板尚无有效内容".into()).into());
        }
        if template
            .paper_id
            .as_deref()
            .is_some_and(|paper| paper != input.paper_id)
        {
            return Err(Error::Invalid("不能采用其他文献的本文模板".into()).into());
        }
        let mut resource = find_by_paper(&tx, &input.paper_id, "structuredReading")?;
        super::require_legacy_imported(&tx, &input.paper_id)?;
        if resource.revision != input.expected_revision {
            return Err(Error::Conflict.into());
        }
        if !input.confirmed && (resource.template_ref.is_some() || resource.content.is_some()) {
            return Err(Error::Invalid("替换已有阅读内容的模板需要用户确认".into()).into());
        }
        let before = resource.clone();
        super::retention::reframe(&mut resource, &template)?;
        resource.template_ref = Some(TemplateRef {
            resource_id: template.id,
            revision: template.revision,
        });
        phase = "validation";
        super::retention::validate_reframed_with(sources, &tx, &before, &resource)?;
        phase = "write";
        resource.revision += 1;
        super::legacy_guard::mark_no_legacy(&tx, &input.paper_id)?;
        let protected: String = tx.query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [&resource.id],
            |r| r.get(0),
        )?;
        let protected =
            super::retention::remap_keys(&resource, &decode::<Vec<String>>(&protected)?);
        repository::persist_revision(&tx, &resource, "human", None, &protected)?;
        tx.execute(
            "update structured_resources set template_id=?2,template_revision=?3 where id=?1",
            params![resource.id, input.template_id, input.template_revision],
        )?;
        drafts::save_receipt(
            &tx,
            &super::Receipt {
                request_id: input.request_id,
                resource_id: resource.id.clone(),
                revision: resource.revision,
                content_hash: hash(&encode(&resource.content)?),
            },
            "bind",
            &fingerprint,
            None,
            None,
        )?;
        Ok((resource, true))
    })();
    let explicit = sources.explicit_settlement();
    let settled = settle(tx, result, phase, explicit);
    boundary::confirm_with(db, settled, explicit, |message| {
        Error::Invalid(message.into())
    })
}
