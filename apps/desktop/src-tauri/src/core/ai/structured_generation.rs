//! Connect generated content to the same scoped text tools used by ordinary chats.
use super::error::AiError;
use crate::agent::context_session::{
    resource_grants,
    sources::{Access, Part, Resource},
};
use research_agent_core::submission::ContextText;
use rusqlite::{Connection, OptionalExtension};
use uuid::Uuid;

#[path = "structured_generation/prompt.rs"]
pub(in crate::ai) mod prompt;

pub(crate) fn paper_grants(
    db: &Connection,
    paper: &str,
    attachments: &[String],
    conversation: Uuid,
    kind: &str,
    source_snapshot: Option<&str>,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<Vec<ContextText>, AiError> {
    // Historical adapter fixtures and old immutable workflows remain readable.
    // Production opens always migrate before preparing a new submission.
    let migrated:bool=db.query_row("select exists(select 1 from sqlite_master where type='table' and name='structured_paper_template_owners')",[],|row|row.get(0))?;
    if !migrated {
        return Ok(Vec::new());
    }
    crate::structured_resources::require_legacy_imported(db, paper).map_err(resource_error)?;
    crate::structured_resources::ensure_paper_resources(db, paper).map_err(resource_error)?;
    let target =
        crate::structured_resources::find_by_paper(db, paper, kind).map_err(resource_error)?;
    let frozen_template = (kind == "structuredReading")
        .then(|| target.template_ref.clone())
        .flatten();
    let mut parts = attachments
        .iter()
        .map(|id| Part {
            resource: Resource::Paper {
                resource_id: paper.into(),
                attachment_id: Some(id.clone()),
                page_index: None,
            },
            label: "来源 PDF".into(),
            selected: true,
            access: Access::ReadOnly,
        })
        .collect::<Vec<_>>();
    parts.push(Part {
        resource: Resource::StructuredObject {
            resource_id: target.id.clone(),
            access: Some(Access::ReadWrite),
        },
        label: target.title.clone(),
        selected: true,
        access: Access::ReadWrite,
    });
    // A new reading conversation owns both stages. Existing user configuration
    // still wins in seed_in, including explicit deselection/read-only choices.
    for other in ["portrait", "structuredReading"] {
        if other == kind {
            continue;
        }
        let resource =
            crate::structured_resources::find_by_paper(db, paper, other).map_err(resource_error)?;
        parts.push(Part {
            resource: Resource::StructuredObject {
                resource_id: resource.id,
                access: Some(Access::ReadWrite),
            },
            label: resource.title,
            selected: true,
            access: Access::ReadWrite,
        });
    }
    let writable_template = {
        let template = crate::structured_resources::ensure_paper_template(db, paper)
            .map_err(resource_error)?;
        let id = template.id.clone();
        parts.push(Part {
            resource: Resource::StructuredObject {
                resource_id: template.id,
                access: Some(Access::ReadWrite),
            },
            label: "本文阅读模板建议".into(),
            selected: true,
            access: Access::ReadWrite,
        });
        Some(id)
    };
    let mut resources = vec![Resource::PaperGroup {
        resource_id: paper.into(),
        parts,
        new_member_access: None,
        group_access: Some(Access::ReadWrite),
    }];
    if let Some(reference) = target.template_ref
        && writable_template.as_deref() != Some(reference.resource_id.as_str())
    {
        let template = crate::structured_resources::get_header(db, &reference.resource_id)
            .map_err(resource_error)?;
        let selected = Resource::StructuredObject {
            resource_id: reference.resource_id,
            access: Some(Access::ReadOnly),
        };
        if template.paper_id.as_deref() == Some(paper) {
            if let Resource::PaperGroup { parts, .. } = &mut resources[0] {
                parts.push(Part {
                    resource: selected,
                    label: template.title,
                    selected: true,
                    access: Access::ReadOnly,
                });
            }
        } else {
            resources.push(selected);
        }
    }
    resources.push(Resource::TemplateCatalog {
        resource_id: crate::structured_resources::TEMPLATE_CATALOG_ID.into(),
        access: Some(Access::ReadOnly),
    });
    crate::agent::context_session::configuration::seed_in(
        db,
        conversation,
        &resources,
        writing_source,
    )
    .map_err(|_| AiError::TemporaryUnavailable)?;
    let resources = crate::agent::context_session::configuration::resources_in(db, conversation)
        .map_err(|_| AiError::TemporaryUnavailable)?;
    let mut grants = resource_grants::grants(db, &resources, writing_source)
        .map_err(|_| AiError::TemporaryUnavailable)?;
    // Retain dynamic group authority just as normal chat does. Resolve its
    // currently selected members for the task's explicit file/target bindings.
    for resource in &resources {
        if let Resource::PaperGroup {
            resource_id,
            new_member_access: Some(_),
            ..
        } = resource
            && resource_id == paper
        {
            let parts =
                crate::agent::context_session::group_scope::parts(db, resource, writing_source)
                    .map_err(|_| AiError::TemporaryUnavailable)?;
            let resolved = Resource::PaperGroup {
                resource_id: resource_id.clone(),
                parts,
                new_member_access: None,
                group_access: None,
            };
            grants.extend(
                resource_grants::grants(db, &[resolved], writing_source)
                    .map_err(|_| AiError::TemporaryUnavailable)?,
            );
        }
    }
    for grant in &mut grants {
        let pinned = (grant.resource_id == target.id)
            .then_some(frozen_template.as_ref())
            .flatten();
        if source_snapshot.is_some() || pinned.is_some() {
            let mut anchor: serde_json::Value =
                serde_json::from_str(grant.anchor.as_deref().unwrap_or("{}"))
                    .map_err(|_| AiError::TemporaryUnavailable)?;
            if let Some(snapshot) = source_snapshot {
                anchor["sourceSnapshotId"] = serde_json::json!(snapshot);
            }
            if let Some(reference) = pinned {
                anchor["frozenTemplateRef"] = serde_json::json!(reference);
            }
            grant.anchor = Some(anchor.to_string());
        }
    }
    grants.sort_by_key(|source| source.resource_id != target.id);
    Ok(grants)
}

pub(in crate::ai) fn require_writable_target(
    grants: &[ContextText],
    target: &str,
) -> Result<(), AiError> {
    if grants.iter().any(|source| {
        source.resource_id == target
            && source
                .anchor
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .is_some_and(|anchor| anchor["contextAccess"] == "readWrite")
    }) {
        return Ok(());
    }
    Err(AiError::InvalidInput(
        "内部任务目标未授予读写权限，请在上下文管理中选择目标并允许读写".into(),
    ))
}

/// Bind the application skill exactly as a composer selection. A disabled or
/// removed skill is not silently reinstalled or replaced by hard-coded rules.
pub(in crate::ai) fn bind_task_skill(
    root: &std::path::Path,
    kind: &str,
    input: &mut research_agent_core::submission::SubmissionInput,
) -> Result<(), AiError> {
    let name = if kind == "portrait" {
        "generate-paper-portrait"
    } else {
        "edit-structured-resources"
    };
    let catalog = crate::agent::skills::application::freeze(root, "")
        .map_err(|_| AiError::TemporaryUnavailable)?;
    let descriptor = catalog
        .iter()
        .find(|skill| skill.descriptor.name == name)
        .ok_or_else(|| {
            AiError::InvalidInput(format!(
                "本任务需要技能 {name}；请先在 Agent 资源设置中启用或导入该技能"
            ))
        })?;
    prompt::bind_skill(input, &descriptor.descriptor);
    let (skills, instructions) =
        crate::agent::skills::injection::freeze_application(root, &input.text, &input.user_input)
            .map_err(|_| AiError::InvalidInput("内部任务技能引用已失效，请重新开始".into()))?;
    input.skills = skills;
    input.skill_instructions = instructions;
    Ok(())
}

fn resource_error(error: crate::structured_resources::Error) -> AiError {
    AiError::InvalidInput(error.to_string())
}

/// Recover only identities established by a trusted workflow. Existing native
/// configuration (including an intentionally empty selection) always wins.
pub(crate) fn seed_existing_context(
    analysis_source: impl Into<Option<super::analysis_source::PaperAnalysisSource>>,
    source: impl Into<Option<crate::ai::paper_reading_source::PaperReadingSource>>,
    db: &Connection,
    conversation: Uuid,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), AiError> {
    seed_existing(
        analysis_source.into(),
        source.into(),
        db,
        conversation,
        writing_source,
    )
}
fn seed_existing(
    analysis_source: Option<super::analysis_source::PaperAnalysisSource>,
    source: Option<crate::ai::paper_reading_source::PaperReadingSource>,
    db: &Connection,
    conversation: Uuid,
    writing_source: Option<crate::agent::writing_resources::WritingResourceSource>,
) -> Result<(), AiError> {
    let configured: bool = db.query_row(
        "select exists(select 1 from agent_context_configurations where conversation_id=?1)",
        [conversation.to_string()],
        |r| r.get(0),
    )?;
    if configured {
        return Ok(());
    }
    let active: bool = db.query_row(
        "select exists(select 1 from agent_conversations where id=?1 and archived=0)",
        [conversation.to_string()],
        |r| r.get(0),
    )?;
    if !active {
        return Ok(());
    }
    let run:Option<String>=db.query_row("select b.ai_run_id from agent_legacy_ai_run_bindings b join ai_runs r on r.id=b.ai_run_id where b.conversation_id=?1 and r.task_kind in ('paper_overview','paper_profile_classification') order by r.created_at desc,r.id desc limit 1",[conversation.to_string()],|r|r.get(0)).optional()?;
    if let Some(run) = run {
        source.ok_or(AiError::CapabilityUnavailable(
            "literature_library.paper_reading_source",
        ))?;
        let run = super::run_repository::get_run(db, &run)?;
        let paper = run
            .scope_items
            .iter()
            .find_map(|item| item.parent_object_id.as_deref())
            .ok_or_else(|| AiError::InvalidInput("旧内部任务缺少可信所属文献".into()))?;
        let attachments = run
            .scope_items
            .iter()
            .filter(|item| item.item_kind == "attachment")
            .map(|item| item.object_id.clone())
            .collect::<Vec<_>>();
        let kind = if run.task_kind == "paper_profile_classification" {
            "portrait"
        } else {
            "structuredReading"
        };
        paper_grants(
            db,
            paper,
            &attachments,
            conversation,
            kind,
            None,
            writing_source,
        )?;
        return Ok(());
    }
    let workflow = if let Some(analysis_source) = analysis_source {
        (analysis_source.conversation_context)(db, conversation).map_err(|e| e.into_ai_error())?
    } else {
        let bound: bool = db.query_row("select exists(select 1 from agent_paper_analysis_bindings b join paper_analysis_workflows w on w.id=b.workflow_id where b.conversation_id=?1)", [conversation.to_string()], |row| row.get(0))?;
        if bound {
            return Err(AiError::CapabilityUnavailable(
                "literature_library.paper_analysis_source",
            ));
        }
        None
    };
    if let Some((paper, snapshot)) = workflow {
        let frozen = super::paper_context::load_snapshot(
            source.ok_or(AiError::CapabilityUnavailable(
                "literature_library.paper_reading_source",
            ))?,
            db,
            &snapshot,
        )?
        .into_reading_result();
        let attachments = frozen
            .attachment_statuses
            .into_iter()
            .map(|item| item.attachment_id)
            .collect::<Vec<_>>();
        paper_grants(
            db,
            &paper,
            &attachments,
            conversation,
            "structuredReading",
            Some(&snapshot),
            writing_source,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "structured_generation/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "structured_generation/completion_tests.rs"]
mod completion_tests;
