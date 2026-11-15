//! Frozen deployed Reading Profile versions become independently identified templates.
use super::{Error, Resource, decode, encode, get, get_version, hash, repository};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};

pub(super) fn definition(id: &str) -> Result<Value, Error> {
    let (title, question) = match id {
        "research_question" => ("研究问题", "论文试图回答的研究问题、目标与需求是什么？"),
        "approach" => (
            "方法／论证路径",
            "论文采用何种方法、系统设计或论证路径来回答研究问题？",
        ),
        "key_result" => ("关键结果", "论文报告哪些主要结果及其原文证据？"),
        "conclusion" => ("结论", "作者据现有证据提出哪些结论及其适用范围？"),
        "limitation" => ("局限", "论文承认或原文证据明确支持哪些局限？"),
        "open_question" => ("开放问题", "论文提出哪些尚待解决的问题或后续研究方向？"),
        "argument" => ("核心论证", "论文的核心论题、前提、推论及论证链条是什么？"),
        "objection" => ("异议／反例", "论文讨论哪些异议、反例及其针对的主张？"),
        "response" => ("回应", "论文如何回应这些异议或反例？"),
        "other" => ("其他", "需要保留的其他原文信息与人工研究记录是什么？"),
        _ => return Err(Error::Invalid(format!("旧阅读框架含未知字段{id}"))),
    };
    Ok(json!({"id":id,"title":title,"question":question}))
}
pub(super) fn import_all(db: &Connection) -> Result<(), Error> {
    let mut statement=db.prepare("select id,version,profile_kind,label,description,field_kinds_json from reading_profiles where review_status='approved' order by id,version")?;
    let profiles = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (id, version, kind, label, description, fields) in profiles {
        let key = format!("reading-profile:{id}");
        if db.query_row("select exists(select 1 from structured_resource_legacy_versions where legacy_id=?1 and legacy_revision=?2)",params![key,version],|r|r.get::<_,bool>(0))?{continue;}
        let field_ids: Vec<String> = decode(&fields)?;
        let fields = field_ids
            .iter()
            .map(|id| definition(id))
            .collect::<Result<Vec<_>, _>>()?;
        let existing:Option<String>=db.query_row("select resource_id from structured_resource_legacy_versions where legacy_id=?1 order by legacy_revision desc limit 1",[&key],|r|r.get(0)).optional()?;
        let resource_id = existing.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        db.execute("insert into structured_resources(id,kind,title) values(?1,'readingTemplate',?2) on conflict(id) do nothing",params![resource_id,label])?;
        let mut target = get(db, &resource_id)?;
        target.revision += 1;
        target.content = Some(
            json!({"kind":"reading_template_v1","schemaVersion":1,"name":label,"fields":fields}),
        );
        target.review_baseline = target.content.clone();
        target.reviewed_field_ids.clear();
        repository::persist_revision(db, &target, "migration", None, &[])?;
        db.execute(
            "update structured_resources set title=?2 where id=?1",
            params![target.id, label],
        )?;
        let audit = json!({"source":{"id":id,"version":version,"profileKind":kind,"label":label,"description":description,"fieldKinds":field_ids},"templateResourceId":target.id,"templateResourceRevision":target.revision,"definitionHash":hash(&encode(&target.content)?)});
        db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) values(?1,?2,?3,?4)",params![key,version,target.id,encode(&audit)?])?;
    }
    Ok(())
}
pub(super) fn resolve(db: &Connection, id: &str, version: i64) -> Result<Resource, Error> {
    let raw:String=db.query_row("select payload_json from structured_resource_legacy_versions where legacy_id=?1 and legacy_revision=?2",params![format!("reading-profile:{id}"),version],|r|r.get(0)).optional()?.ok_or_else(||Error::Invalid(format!("旧阅读框架{id}@{version}没有准确版本映射")))?;
    let audit: Value = decode(&raw)?;
    get_version(
        db,
        audit["templateResourceId"]
            .as_str()
            .ok_or_else(|| Error::Invalid("旧框架映射身份无效".into()))?,
        audit["templateResourceRevision"]
            .as_i64()
            .ok_or_else(|| Error::Invalid("旧框架映射版本无效".into()))?,
    )
}
