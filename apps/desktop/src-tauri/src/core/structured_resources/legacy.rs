//! One-time import preserves legacy candidates and human reading bodies verbatim.
use super::{
    Error, LegacyImportDraft, Resource, TemplateRef, decode, encode, ensure_paper_resources,
    find_by_paper, get, hash, list, repository,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
#[path = "legacy/quotes.rs"]
mod quotes;
#[path = "legacy/template_import.rs"]
mod template_import;

pub(crate) fn import_legacy(
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: LegacyImportDraft,
) -> Result<Vec<Resource>, Error> {
    import_legacy_with_evidence(&reading_source, db, input)
}

pub(crate) fn import_legacy_with_evidence(
    reading_source: &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence,
    db: &mut Connection,
    input: LegacyImportDraft,
) -> Result<Vec<Resource>, Error> {
    let tx = db.transaction()?;
    import_catalog(&tx, input.catalog.as_ref())?;
    if let Some(paper) = input.paper_id.as_deref() {
        ensure_paper_resources(&tx, paper)?;
        let key = format!("paper:{paper}");
        if !imported(&tx, &key)? {
            super::legacy_guard::require_untouched(&tx, paper)?;
            import_paper(reading_source, &tx, paper, input.presentation.as_ref())?;
            mark(&tx, &key)?;
        }
    }
    tx.commit()?;
    list(db, input.paper_id.as_deref())
}
pub(super) fn import_catalog(db: &Connection, catalog: Option<&Value>) -> Result<(), Error> {
    super::legacy_profiles::import_all(db)?;
    if let Some(catalog) = catalog {
        for template in catalog["templates"]
            .as_array()
            .ok_or_else(|| Error::Invalid("旧模板目录格式无效".into()))?
        {
            let id = template["id"]
                .as_str()
                .ok_or_else(|| Error::Invalid("旧模板缺少身份".into()))?;
            let name = template["label"].as_str().unwrap_or("迁移模板");
            imported_template(db, &format!("catalog:{id}"), name, &template["fields"])?;
        }
    }
    Ok(())
}

pub(super) fn imported(db: &Connection, key: &str) -> Result<bool, Error> {
    Ok(db.query_row(
        "select exists(select 1 from structured_resource_legacy_imports where import_key=?1)",
        [key],
        |r| r.get(0),
    )?)
}
pub(super) fn mark(db: &Connection, key: &str) -> Result<(), Error> {
    db.execute("insert into structured_resource_legacy_imports(import_key) values(?1) on conflict(import_key) do nothing",[key])?;
    Ok(())
}

pub(super) fn imported_template(
    db: &Connection,
    key: &str,
    name: &str,
    fields: &Value,
) -> Result<Resource, Error> {
    let legacy = format!("template:{key}");
    if let Some(id)=db.query_row("select resource_id from structured_resource_legacy_versions where legacy_id=?1 and legacy_revision=1",[&legacy],|r|r.get::<_,String>(0)).optional()?{return get(db,&id);}
    let mut fields = fields
        .as_array()
        .cloned()
        .ok_or_else(|| Error::Invalid("旧模板字段无效".into()))?;
    for field in &mut fields {
        if let Some(object) = field.as_object_mut() {
            object.remove("claimIds");
        }
    }
    let value = json!({"kind":"reading_template_v1","schemaVersion":1,"name":name,"fields":fields});
    let fingerprint = hash(&encode(&value)?);
    if let Some(id)=db.query_row("select resource_id from structured_resource_legacy_versions where legacy_id=?1 and legacy_revision=1",[format!("template-content:{fingerprint}")],|r|r.get::<_,String>(0)).optional()?{
        db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) values(?1,1,?2,?3)",params![legacy,id,encode(&value)?])?;return get(db,&id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    db.execute(
        "insert into structured_resources(id,kind,title) values(?1,'readingTemplate',?2)",
        params![id, name],
    )?;
    let mut resource = get(db, &id)?;
    resource.content = Some(value.clone());
    resource.review_baseline = Some(value.clone());
    resource.revision = 1;
    repository::persist_revision(db, &resource, "migration", None, &[])?;
    for legacy in [legacy, format!("template-content:{fingerprint}")] {
        db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) values(?1,1,?2,?3)",params![legacy,id,encode(&value)?])?;
    }
    Ok(resource)
}

struct Candidate {
    id: String,
    kind: String,
    payload: Value,
    state: String,
    bundle: String,
}
fn import_paper(
    reading_source: &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence,
    db: &Connection,
    paper: &str,
    presentation: Option<&super::LegacyPresentation>,
) -> Result<(), Error> {
    let mut stmt=db.prepare("select c.id,c.current_revision,c.candidate_kind,r.payload_json,c.review_state,c.evidence_bundle_id from ai_candidates c join ai_candidate_revisions r on r.candidate_id=c.id and r.revision=c.current_revision where c.candidate_kind in ('paper_portrait_v1','paper_overview_unit_v1') and exists(select 1 from ai_evidence_items e where e.bundle_id=c.evidence_bundle_id and e.paper_id=?1) and not exists(select 1 from ai_evidence_items e where e.bundle_id=c.evidence_bundle_id and e.paper_id!=?1) order by c.created_at desc,c.id desc")?;
    let rows = stmt
        .query_map([paper], |r| {
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
    let candidates = rows
        .into_iter()
        .map(|r| {
            Ok(Candidate {
                id: r.0,
                kind: r.2,
                payload: decode(&r.3)?,
                state: r.4,
                bundle: r.5,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    for candidate in &candidates {
        let kind = if candidate.kind == "paper_portrait_v1" {
            "portrait"
        } else {
            "structuredReading"
        };
        let resource = find_by_paper(db, paper, kind)?;
        db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) select candidate_id,revision,?2,payload_json from ai_candidate_revisions where candidate_id=?1 on conflict(legacy_id,legacy_revision) do nothing",params![candidate.id,resource.id])?;
    }
    let adopted:Option<String>=db.query_row("select d.candidate_id from ai_review_decisions d join ai_candidates c on c.id=d.candidate_id where c.candidate_kind='paper_portrait_v1' and d.action in ('accept_unchanged','accept_after_edit') and exists(select 1 from ai_evidence_items e where e.bundle_id=c.evidence_bundle_id and e.paper_id=?1) order by d.created_at desc,d.id desc limit 1",[paper],|r|r.get(0)).optional()?;
    let portrait = adopted
        .as_ref()
        .and_then(|id| candidates.iter().find(|c| &c.id == id))
        .or_else(|| {
            candidates
                .iter()
                .find(|c| c.kind == "paper_portrait_v1" && c.state != "rejected")
        });
    let latest = candidates
        .iter()
        .find(|c| c.kind == "paper_overview_unit_v1" && c.state != "rejected");
    let template = template_import::formal(db, paper, &candidates, presentation, latest)?;
    if let Some(candidate) = portrait {
        let mut resource = find_by_paper(db, paper, "portrait")?;
        if resource.revision == 0 {
            resource.content = Some(candidate.payload.clone());
            resource.revision = 1;
            if candidate.state.starts_with("accepted_") {
                resource.review_baseline = resource.content.clone();
                resource.reviewed_field_ids = super::content::fields(resource.content.as_ref())
                    .into_keys()
                    .collect();
            }
            resource.template_ref = template.as_ref().map(|t| TemplateRef {
                resource_id: t.id.clone(),
                revision: t.revision,
            });
            repository::persist_revision(db, &resource, "migration", Some(&candidate.bundle), &[])?;
        }
    }
    let mut resource = find_by_paper(db, paper, "structuredReading")?;
    if resource.revision > 0 {
        return Ok(());
    }
    let mut payload=latest.map(|c|c.payload.clone()).unwrap_or_else(||json!({"kind":"paper_overview_unit_v1","schemaVersion":1,"profileId":"generic_overview_v1","fields":(["research_question","approach","key_result","conclusion","limitation","open_question"].map(|field|json!({"field":field,"status":"not_found","statements":[]})))}));
    let mut fields = if let Some(presentation) = presentation {
        presentation
            .fields
            .iter()
            .filter_map(|field| {
                candidates
                    .iter()
                    .find(|c| c.id == field.candidate_id)
                    .and_then(|c| c.payload["fields"].as_array())
                    .and_then(|fields| fields.iter().find(|v| v["field"] == field.field_id))
                    .cloned()
            })
            .collect::<Vec<_>>()
    } else {
        payload["fields"].as_array().cloned().unwrap_or_default()
    };
    let mut field_sources = std::collections::BTreeMap::<String, Vec<String>>::new();
    for field in &fields {
        let id = field["field"].as_str().unwrap_or_default();
        let candidate = presentation
            .and_then(|p| p.fields.iter().find(|f| f.field_id == id))
            .and_then(|f| candidates.iter().find(|c| c.id == f.candidate_id))
            .or(latest);
        if let Some(candidate) = candidate {
            field_sources.insert(id.into(), vec![candidate.bundle.clone()]);
        }
    }
    let mut stmt=db.prepare("select u.id,u.body,u.unit_kind,u.revision,f.field_id from structured_reading_units u left join reading_unit_template_fields f on f.reading_unit_id=u.id where u.paper_id=?1 and u.reverted_at is null order by u.updated_at,u.id")?;
    let units = stmt
        .query_map([paper], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut has_body = latest.is_some() && !fields.is_empty();
    for (id, body, kind, revision, field_id) in units {
        db.execute("insert into structured_resource_legacy_versions(legacy_id,legacy_revision,resource_id,payload_json) values(?1,?2,?3,?4) on conflict(legacy_id,legacy_revision) do nothing",params![format!("reading-unit:{id}"),revision,resource.id,encode(&json!({"id":id,"body":body,"unitKind":kind,"revision":revision}))?])?;
        if presentation.is_some_and(|p| p.hidden_unit_ids.contains(&id)) {
            continue;
        }
        has_body = true;
        let field_id = field_id.unwrap_or(kind);
        let target = fields.iter().position(|field| field["field"] == field_id);
        let mut anchor_stmt=db.prepare("select s.id,s.quote_exact from reading_unit_evidence_links l join source_anchors s on s.id=l.anchor_id where l.reading_unit_id=?1 order by l.created_at,l.id")?;
        let anchors = anchor_stmt
            .query_map([&id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut statement = json!({"id":format!("legacy-{id}"),"text":body,"epistemicRole":"working_note","evidenceItemIds":anchors.iter().map(|(id,_)|id).collect::<Vec<_>>()});
        if !anchors.is_empty() {
            statement["evidenceQuotes"] = json!(
                anchors
                    .iter()
                    .map(|(id, quote)| json!({"evidenceItemId":id,"quote":quote}))
                    .collect::<Vec<_>>()
            );
            field_sources.insert(
                field_id.clone(),
                anchors
                    .iter()
                    .map(|(id, _)| format!("anchor:{id}"))
                    .collect(),
            );
        } else if let Some(index) = target {
            let old = &fields[index]["statements"];
            if let Some(statements) = old.as_array() {
                let ids = statements
                    .iter()
                    .flat_map(|s| s["evidenceItemIds"].as_array().cloned().unwrap_or_default())
                    .collect::<Vec<_>>();
                statement["evidenceItemIds"] = json!(ids);
                let quotes = statements
                    .iter()
                    .flat_map(|s| s["evidenceQuotes"].as_array().cloned().unwrap_or_default())
                    .collect::<Vec<_>>();
                if !quotes.is_empty() {
                    statement["evidenceQuotes"] = json!(quotes);
                }
            }
        }
        if let Some(index) = target {
            fields[index]["statements"] = json!([statement]);
            fields[index]["status"] = json!("filled");
        } else {
            fields.push(json!({"field":field_id,"status":"filled","statements":[statement]}));
        }
        let reviewed:bool=db.query_row("select exists(select 1 from ai_overview_field_confirmations f left join ai_overview_confirmation_undos u on u.batch_id=f.batch_id left join ai_overview_field_review_resets r on r.field_confirmation_id=f.id where f.target_reading_unit_id=?1 and f.target_applied_revision=?2 and u.id is null and r.id is null)",params![id,revision],|r|r.get(0))?;
        if reviewed {
            resource.reviewed_field_ids.push(field_id);
        }
    }
    if has_body {
        if template.is_none() && payload["profileId"] == "generic_overview_v1" {
            // A legacy presentation may omit empty/hidden generic fields, while
            // free-form human units can add other IDs. Keep the fixed active
            // schema editable and preserve those extra bodies as inactive fields.
            let mut by_id = fields
                .into_iter()
                .map(|field| {
                    (
                        field["field"].as_str().unwrap_or_default().to_owned(),
                        field,
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>();
            fields = [
                "research_question",
                "approach",
                "key_result",
                "conclusion",
                "limitation",
                "open_question",
            ]
            .into_iter()
            .map(|id| {
                by_id
                    .remove(id)
                    .unwrap_or_else(|| json!({"field":id,"status":"not_found","statements":[]}))
            })
            .collect();
            let mut retained = payload["retainedFields"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            retained.extend(by_id.into_values());
            if !retained.is_empty() {
                payload["retainedFields"] = json!(retained);
            }
        }
        payload["fields"] = json!(fields);
        disambiguate_statement_ids(&mut payload);
        resource.content = Some(payload);
        resource.revision = 1;
        if template.is_some()
            && let Some(content) = resource.content.as_mut()
        {
            quotes::fill(reading_source, db, paper, content, &field_sources)?;
        }
        resource.reviewed_field_ids =
            super::retention::remap_keys(&resource, &resource.reviewed_field_ids);
        if !resource.reviewed_field_ids.is_empty() {
            let mut baseline = None;
            super::content::replace_fields(
                &mut baseline,
                resource.content.as_ref(),
                &resource.reviewed_field_ids,
                true,
            );
            resource.review_baseline = baseline;
        }
        resource.template_ref = template.as_ref().map(|t| TemplateRef {
            resource_id: t.id.clone(),
            revision: t.revision,
        });
        if let Some(template) = template.as_ref() {
            super::retention::reframe(&mut resource, template)?;
        }
        repository::persist_revision(
            db,
            &resource,
            "migration",
            latest.map(|c| c.bundle.as_str()),
            &[],
        )?;
        for (field, sources) in field_sources {
            let field = super::retention::remap_keys(&resource, &[field]).remove(0);
            super::provenance::set_field_sources(
                db,
                &resource.id,
                resource.revision,
                &field,
                &sources,
            )?;
        }
    } else if let Some(template) = template.as_ref() {
        resource.template_ref = Some(TemplateRef {
            resource_id: template.id.clone(),
            revision: template.revision,
        });
        resource.revision = 1;
        repository::persist_revision(db, &resource, "migration", None, &[])?;
    }
    if template.is_none()
        && let Some(portrait) = portrait
    {
        template_import::proposal(db, paper, portrait)?;
    }
    Ok(())
}

// Legacy field presentation can combine independent model results whose local
// statement IDs coincide. Namespace only collisions; immutable inputs stay intact.
fn disambiguate_statement_ids(value: &mut Value) {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for key in ["fields", "retainedFields"] {
        for field in value[key].as_array().into_iter().flatten() {
            for statement in field["statements"].as_array().into_iter().flatten() {
                if let Some(id) = statement["id"].as_str() {
                    *counts.entry(id.into()).or_default() += 1;
                }
            }
        }
    }
    for key in ["fields", "retainedFields"] {
        for field in value
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            let field_id = field["field"].as_str().unwrap_or_default().to_owned();
            for (index, statement) in field
                .get_mut("statements")
                .and_then(Value::as_array_mut)
                .into_iter()
                .flatten()
                .enumerate()
            {
                if let Some(id) = statement["id"].as_str()
                    && counts.get(id).is_some_and(|count| *count > 1)
                {
                    statement["id"] = json!(format!(
                        "legacy-{}",
                        hash(&format!("{key}:{field_id}:{id}:{index}"))
                    ));
                }
            }
        }
    }
}
