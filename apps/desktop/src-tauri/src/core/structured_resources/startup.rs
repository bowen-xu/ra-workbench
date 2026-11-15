//! Renderer preferences join the metadata-only native legacy scan before authoring opens.
use super::{
    Error, LegacyImportDraft, StartupImportDraft, StartupImportResult, legacy, legacy_guard,
};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn import_startup(
    reading_source: impl Into<Option<crate::ai::paper_reading_source::PaperReadingSource>>,
    db: &mut Connection,
    input: StartupImportDraft,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<StartupImportResult, Error> {
    let reading_source = reading_source.into();
    import_startup_with_evidence(
        reading_source.as_ref().map(|source| {
            source as &dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence
        }),
        db,
        input,
        writing_source,
    )
}

pub(crate) fn import_startup_with_evidence(
    reading_source: Option<&dyn crate::ai::structured_resource_validation::ExplicitReadingEvidence>,
    db: &mut Connection,
    input: StartupImportDraft,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<StartupImportResult, Error> {
    let mut preferences = BTreeMap::new();
    for preference in input.presentations {
        if preferences
            .insert(preference.paper_id, preference.presentation)
            .is_some()
        {
            return Err(Error::Invalid("启动迁移包含重复的文献展示偏好".into()));
        }
    }
    let before = global_count(db)?;
    let tx = db.transaction()?;
    legacy::import_catalog(&tx, input.catalog.as_ref())?;
    tx.commit()?;
    let imported_template_count = (global_count(db)? - before) as usize;
    let mut statement =
        db.prepare("select id from bibliographic_items where trashed_at is null order by id")?;
    let candidates = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let mut papers = BTreeSet::new();
    for paper in candidates {
        if preferences.contains_key(&paper) || legacy_guard::has_legacy_data(db, &paper)? {
            papers.insert(paper);
        }
    }
    let mut imported_paper_count = 0;
    let mut ready_papers = Vec::new();
    let mut deferred_paper_ids = Vec::new();
    for paper in &papers {
        let was_imported = legacy::imported(db, &format!("paper:{paper}"))?;
        if was_imported {
            ready_papers.push(paper.clone());
            continue;
        }
        let Some(reading_source) = reading_source else {
            // Keep the original rows/preferences and recovery predicate. The
            // selected owner performs this same import on a later startup.
            deferred_paper_ids.push(paper.clone());
            continue;
        };
        super::import_legacy_with_evidence(
            reading_source,
            db,
            LegacyImportDraft {
                request_id: format!("startup:{paper}"),
                catalog: None,
                paper_id: Some(paper.clone()),
                presentation: preferences.remove(paper),
            },
        )?;
        if !was_imported {
            imported_paper_count += 1;
        }
        ready_papers.push(paper.clone());
    }
    let mut imported_context_count = 0;
    let mut deferred_context_ids = Vec::new();
    for context in input.contexts {
        let before: Option<i64> = db
            .query_row(
                "select revision from agent_context_configurations where conversation_id=?1",
                [context.conversation_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if before.is_none()
            && writing_source.is_none()
            && needs_writing_owner(&context.state)
            && db.query_row(
                "select exists(select 1 from agent_conversations where id=?1)",
                [context.conversation_id.to_string()],
                |row| row.get::<_, bool>(0),
            )?
        {
            deferred_context_ids.push(context.conversation_id);
            continue;
        }
        crate::agent::context_session::configuration::import_legacy_in(
            db,
            context.conversation_id,
            &context.state,
            writing_source,
        )
        .map_err(|error| Error::Invalid(format!("对话上下文偏好迁移失败：{error}")))?;
        if before.is_none()&&db.query_row("select exists(select 1 from agent_context_configurations where conversation_id=?1)",[context.conversation_id.to_string()],|row|row.get::<_,bool>(0))?{imported_context_count+=1;}
    }
    Ok(StartupImportResult {
        paper_ids: ready_papers,
        imported_paper_count,
        imported_template_count,
        imported_context_count,
        deferred_paper_ids,
        deferred_context_ids,
    })
}

fn needs_writing_owner(state: &serde_json::Value) -> bool {
    use crate::agent::context_session::sources::Resource;
    ["persistent", "temporary"].into_iter().any(|key| {
        state[key].as_array().is_some_and(|selections| {
            selections.iter().any(|selection| {
                matches!(
                    serde_json::from_value::<Resource>(selection["resource"].clone()),
                    Ok(Resource::Manuscript { .. } | Resource::ManuscriptFile { .. })
                )
            })
        })
    })
}
fn global_count(db: &Connection) -> Result<i64, Error> {
    Ok(db.query_row("select count(*) from structured_resources r where r.kind='readingTemplate' and not exists(select 1 from structured_paper_template_owners o where o.template_id=r.id)",[],|row|row.get(0))?)
}
