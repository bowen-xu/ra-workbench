//! Existing displayed reading snapshots take precedence over later portrait suggestions.
use super::{Candidate, Error, Resource, encode, find_by_paper, hash, json, repository};
use rusqlite::{Connection, params};

pub(super) fn formal(
    db: &Connection,
    paper: &str,
    candidates: &[Candidate],
    presentation: Option<&crate::structured_resources::LegacyPresentation>,
    latest: Option<&Candidate>,
) -> Result<Option<Resource>, Error> {
    let selected = if let Some(presentation) = presentation {
        presentation
            .fields
            .iter()
            .filter_map(|field| {
                candidates
                    .iter()
                    .find(|candidate| candidate.id == field.candidate_id)
                    .map(|candidate| (candidate, field.field_id.as_str()))
            })
            .collect::<Vec<_>>()
    } else {
        latest
            .into_iter()
            .flat_map(|candidate| {
                candidate.payload["fields"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(move |field| field["field"].as_str().map(|id| (candidate, id)))
            })
            .collect()
    };
    let custom = selected
        .iter()
        .any(|(candidate, _)| candidate.payload["paperTemplate"]["fields"].is_array());
    let mut refs = vec![];
    for (candidate, _) in &selected {
        for reference in references(&candidate.payload)? {
            if !refs.contains(&reference) {
                refs.push(reference);
            }
        }
    }
    if selected.is_empty() {
        use rusqlite::OptionalExtension;
        if let Some(raw) = db
            .query_row(
                "select profile_refs_json from paper_reading_profile_selections where paper_id=?1",
                [paper],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            refs = parse_refs(
                &serde_json::from_str::<serde_json::Value>(&raw)
                    .map_err(|_| Error::Invalid("旧阅读框架选择无效".into()))?,
            )?;
        }
        if refs.is_empty()&&db.query_row("select exists(select 1 from structured_reading_units where paper_id=?1 and reverted_at is null)",[paper],|row|row.get::<_,bool>(0))?{refs.push(("generic_overview".into(),1));}
    }
    if !custom && refs.is_empty() {
        return Ok(None);
    }
    let mut definitions = vec![];
    let mut seen = std::collections::HashSet::new();
    for (candidate, id) in &selected {
        if !seen.insert((*id).to_owned()) {
            continue;
        }
        let definition = candidate.payload["paperTemplate"]["fields"]
            .as_array()
            .and_then(|fields| fields.iter().find(|field| field["id"] == *id))
            .map(|field| {
                crate::structured_resources::validation::normalized_fields(&json!([field]))[0]
                    .clone()
            })
            .map(Ok)
            .unwrap_or_else(|| profile_definition(db, &candidate.payload, id))?;
        definitions.push(definition);
    }
    if selected.is_empty() {
        for (id, version) in &refs {
            let template = crate::structured_resources::legacy_profiles::resolve(db, id, *version)?;
            for field in template
                .content
                .as_ref()
                .and_then(|value| value["fields"].as_array())
                .into_iter()
                .flatten()
            {
                if let Some(id) = field["id"].as_str()
                    && seen.insert(id.to_owned())
                {
                    definitions.push(field.clone());
                }
            }
        }
    }
    let target = if !custom && refs.len() == 1 {
        crate::structured_resources::legacy_profiles::resolve(db, &refs[0].0, refs[0].1)?
    } else {
        if definitions.is_empty() {
            return Ok(None);
        }
        if definitions.len() > 20 {
            return Err(Error::Invalid(
                "旧混合阅读模板超过20字段，无法安全迁移；原候选与正文已保留，请先恢复该展示配置"
                    .into(),
            ));
        }
        let mut target = find_by_paper(db, paper, "readingTemplate")?;
        if target.revision == 0 {
            target.content = Some(
                json!({"kind":"reading_template_v1","schemaVersion":1,"name":"本文阅读模板","fields":definitions}),
            );
            target.review_baseline = target.content.clone();
            target.revision = 1;
            repository::persist_revision(db, &target, "migration", None, &[])?;
        }
        target
    };
    let audit = json!({"profileRefs":refs.iter().map(|(id,version)|json!({"profileId":id,"profileVersion":version})).collect::<Vec<_>>(),"fields":definitions,"templateResourceId":target.id,"templateResourceRevision":target.revision});
    db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) values(?1,1,?2,?3) on conflict(legacy_id,legacy_revision) do nothing",params![format!("paper-profile-adoption:{paper}"),target.id,encode(&audit)?])?;
    Ok(Some(target))
}
fn parse_refs(value: &serde_json::Value) -> Result<Vec<(String, i64)>, Error> {
    value
        .as_array()
        .ok_or_else(|| Error::Invalid("旧阅读框架引用无效".into()))?
        .iter()
        .map(|reference| {
            Ok((
                reference["profileId"]
                    .as_str()
                    .ok_or_else(|| Error::Invalid("旧阅读框架身份无效".into()))?
                    .to_owned(),
                reference["profileVersion"]
                    .as_i64()
                    .filter(|version| *version > 0)
                    .ok_or_else(|| Error::Invalid("旧阅读框架版本无效".into()))?,
            ))
        })
        .collect()
}
fn references(value: &serde_json::Value) -> Result<Vec<(String, i64)>, Error> {
    if value["profileId"] == "generic_overview_v1" {
        Ok(vec![("generic_overview".into(), 1)])
    } else if let Some(refs) = value.get("profileRefs") {
        parse_refs(refs)
    } else {
        Ok(vec![])
    }
}
fn profile_definition(
    db: &Connection,
    value: &serde_json::Value,
    id: &str,
) -> Result<serde_json::Value, Error> {
    for (profile, version) in references(value)? {
        let template =
            crate::structured_resources::legacy_profiles::resolve(db, &profile, version)?;
        if let Some(field) = template
            .content
            .as_ref()
            .and_then(|value| value["fields"].as_array())
            .and_then(|fields| fields.iter().find(|field| field["id"] == id))
        {
            return Ok(field.clone());
        }
    }
    Err(Error::Invalid(format!(
        "旧字段{id}没有对应冻结框架版本中的定义"
    )))
}

pub(super) fn proposal(db: &Connection, paper: &str, portrait: &Candidate) -> Result<(), Error> {
    let target = find_by_paper(db, paper, "readingTemplate")?;
    if target.revision != 0 {
        return Ok(());
    }
    let raw = encode(
        &json!({"kind":"reading_template_v1","schemaVersion":1,"name":"本文阅读模板","fields":crate::structured_resources::validation::normalized_fields(&portrait.payload["template"])}),
    )?;
    let id = uuid::Uuid::new_v4().to_string();
    db.execute("insert into structured_resource_drafts(id,resource_id,creator_scope,base_revision,revision,content,content_hash,source_snapshot_id) values(?1,?2,'human',0,1,?3,?4,?5) on conflict(resource_id,creator_scope) do nothing",params![id,target.id,raw,hash(&raw),portrait.bundle])?;
    Ok(())
}

#[cfg(test)]
#[test]
fn mixed_display_definitions_follow_each_selected_field_snapshot() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Test','now','now')",[]).unwrap();
    let candidates=[("a","A问题","A设计"),("b","B问题","B设计")].map(|(id,question,design)|Candidate{id:id.into(),kind:"paper_overview_unit_v1".into(),payload:json!({"paperTemplate":{"fields":[{"id":"question","title":question,"question":"研究的问题","claimIds":[]},{"id":"design","title":design,"question":"设计内容","claimIds":[]}]}}),state:"unreviewed".into(),bundle:id.into()});
    let selection = crate::structured_resources::LegacyPresentation {
        fields: vec![
            crate::structured_resources::LegacyField {
                candidate_id: "b".into(),
                field_id: "design".into(),
            },
            crate::structured_resources::LegacyField {
                candidate_id: "a".into(),
                field_id: "question".into(),
            },
        ],
        hidden_unit_ids: vec![],
    };
    let imported = formal(&db, "paper", &candidates, Some(&selection), None)
        .unwrap()
        .unwrap();
    let fields = &imported.content.as_ref().unwrap()["fields"];
    assert_eq!(fields[0]["id"], "design");
    assert_eq!(fields[0]["title"], "B设计");
    assert_eq!(fields[1]["title"], "A问题");
}
