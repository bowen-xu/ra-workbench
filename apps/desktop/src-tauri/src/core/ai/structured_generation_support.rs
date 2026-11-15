//! Closed shared facts for paper workflow policy on the caller's original guard.
use super::{candidate_contracts::AiEvidenceBundle, error::AiError, generation_support::AiRun};
use crate::{
    agent::context_session::{resource_grants, sources::Resource},
    storage::AppStore,
};
use research_agent_core::submission::ContextText;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use uuid::Uuid;

pub(crate) struct StructuredTarget {
    pub id: String,
    pub source_uri: String,
}

pub(crate) use research_workbench_native_sdk::analysis_workflow::prompt::PromptResourceV1 as PromptResource;

pub(crate) fn prompt_resource(source: &ContextText) -> Option<PromptResource> {
    resource_grants::resource(source).map(|resource| match resource {
        Resource::Paper {
            resource_id,
            attachment_id,
            ..
        } => PromptResource::Paper {
            resource_id,
            attachment_id,
        },
        Resource::TemplateCatalog { .. } => PromptResource::TemplateCatalog,
        _ => PromptResource::Other,
    })
}

pub(crate) fn initial_context(context: &[ContextText]) -> Result<Option<String>, AiError> {
    crate::agent::execution::resource_context::initial(context)
        .map_err(|_| AiError::TemporaryUnavailable)
}

pub(crate) fn decorate_submission(
    input: &mut super::generation_support::ProviderInput,
    submission: &research_agent_core::submission::SubmissionInput,
    system: &str,
) -> Result<(), AiError> {
    use super::generation_support::{ProviderMessage, ProviderMessageRole};
    input.audit_text = submission.text.clone();
    input.messages = vec![ProviderMessage {
        role: ProviderMessageRole::User,
        text: submission.text.clone(),
        images: vec![],
    }];
    input.system_text = Some(system.into());
    if let Some(scope) = initial_context(&submission.context)? {
        input
            .system_text
            .as_mut()
            .ok_or(AiError::TemporaryUnavailable)?
            .push_str(&format!("\n\n{scope}"));
    }
    Ok(())
}

pub(crate) fn require_writable_target(grants: &[ContextText], target: &str) -> Result<(), AiError> {
    super::structured_generation::require_writable_target(grants, target)
}

pub(crate) struct StructuredGenerationFacts<'a> {
    db: &'a Connection,
    store: &'a AppStore,
}

impl<'a> StructuredGenerationFacts<'a> {
    pub(crate) fn new(db: &'a Connection, store: &'a AppStore) -> Self {
        Self { db, store }
    }

    pub(crate) fn present(&self) -> Result<bool, AiError> {
        self.db.query_row("select exists(select 1 from sqlite_master where type='table' and name='structured_resources')", [], |r| r.get(0)).map_err(Into::into)
    }

    pub(crate) fn run(
        &self,
        conversation: Uuid,
        request: Uuid,
        kinds: [&str; 2],
    ) -> Result<Option<AiRun>, AiError> {
        let run: Option<String> = self.db.query_row("select r.id from ai_runs r join agent_legacy_ai_run_bindings b on b.ai_run_id=r.id where b.conversation_id=?1 and b.request_id=?2 and r.task_kind in (?3,?4)", params![conversation.to_string(),request.to_string(),kinds[0],kinds[1]], |r|r.get(0)).optional()?;
        run.map(|id| super::run_repository::get_run(self.db, &id))
            .transpose()
    }

    pub(crate) fn bundle(&self, run: &str) -> Result<(String, AiEvidenceBundle), AiError> {
        let id: String = self.db.query_row(
            "select id from ai_evidence_bundles where run_id=?1",
            [run],
            |r| r.get(0),
        )?;
        let bundle = super::evidence_repository::load_bundle(self.db, &id)?;
        Ok((id, bundle))
    }

    pub(crate) fn grants(
        &self,
        paper: &str,
        attachments: &[String],
        conversation: Uuid,
        kind: &str,
        snapshot: &str,
    ) -> Result<Vec<ContextText>, AiError> {
        super::structured_generation::paper_grants(
            self.db,
            paper,
            attachments,
            conversation,
            kind,
            Some(snapshot),
            self.store.writing_resource_source(),
        )
    }

    pub(crate) fn target(&self, paper: &str, kind: &str) -> Result<StructuredTarget, AiError> {
        let value = crate::structured_resources::find_by_paper(self.db, paper, kind)
            .map_err(resource_error)?;
        Ok(StructuredTarget {
            id: value.id,
            source_uri: value.source_uri,
        })
    }

    pub(crate) fn prompt_templates(&self) -> Result<(String, String), AiError> {
        let root = self.store.agent_configuration_directory();
        let task = crate::system_documents::prompt(root, "structured-task")
            .map_err(AiError::InvalidInput)?;
        let contract = crate::system_documents::prompt(root, "structured-reading-contract")
            .map_err(AiError::InvalidInput)?;
        Ok((task, contract))
    }

    pub(crate) fn writable_template(
        &self,
        paper: &str,
        grants: &[ContextText],
    ) -> Result<Option<(String, String)>, AiError> {
        let template = self.target(paper, "readingTemplate")?;
        Ok(require_writable_target(grants, &template.id)
            .is_ok()
            .then_some((template.id, template.source_uri)))
    }

    /// The existing Core resource receipts remain the authority for a completed
    /// submission. Read in the original order, including short-circuit failures.
    pub(crate) fn completion_receipt(
        &self,
        resource: &str,
        template: Option<&str>,
        conversation: Uuid,
        request: Uuid,
    ) -> Result<
        (
            Value,
            research_workbench_native_sdk::analysis_workflow::submission::CompletionTemplateV1,
        ),
        AiError,
    > {
        use research_workbench_native_sdk::analysis_workflow::submission::CompletionTemplateV1;
        if self
            .latest_status(resource, conversation, request)?
            .as_deref()
            != Some("applied")
            || self.has_owned_draft(resource, conversation)?
        {
            return Err(AiError::InvalidInput(
                "本轮目标尚有未成功应用的修改；修复草稿已保留，请继续修复后完成".into(),
            ));
        }
        let content = self
            .applied_content(conversation, request, resource)?
            .ok_or_else(|| {
                AiError::InvalidInput(
                    "本轮尚未成功写入目标资源；修复草稿仍保留，可在内部对话继续修改".into(),
                )
            })?;
        let template = match template {
            Some(id) => {
                let content = self.template_content(id)?;
                if self.template_has_draft(id, conversation)? {
                    return Err(AiError::InvalidInput(
                        "本文阅读模板仍有未应用的修复草稿；请修复后继续".into(),
                    ));
                }
                CompletionTemplateV1::Required { content }
            }
            None => CompletionTemplateV1::NotRequired,
        };
        Ok((content, template))
    }

    pub(crate) fn latest_status(
        &self,
        resource: &str,
        conversation: Uuid,
        request: Uuid,
    ) -> Result<Option<String>, AiError> {
        self.db.query_row("select json_extract(result_json,'$.status') from structured_resource_operations where resource_id=?1 and conversation_id=?2 and submission_id=?3 order by rowid desc limit 1", params![resource,conversation.to_string(),request.to_string()], |r|r.get(0)).optional().map_err(Into::into)
    }

    pub(crate) fn has_owned_draft(
        &self,
        resource: &str,
        conversation: Uuid,
    ) -> Result<bool, AiError> {
        crate::structured_resources::has_owned_draft(self.db, resource, &conversation.to_string())
            .map_err(resource_error)
    }

    pub(crate) fn applied_content(
        &self,
        conversation: Uuid,
        request: Uuid,
        resource: &str,
    ) -> Result<Option<Value>, AiError> {
        crate::structured_resources::applied_content_for_submission(
            self.db,
            &conversation.to_string(),
            &request.to_string(),
            resource,
        )
        .map_err(resource_error)
    }

    pub(crate) fn template_content(&self, resource: &str) -> Result<Option<Value>, AiError> {
        crate::structured_resources::get(self.db, resource)
            .map(|value| value.content)
            .map_err(resource_error)
    }

    pub(crate) fn template_has_draft(
        &self,
        resource: &str,
        conversation: Uuid,
    ) -> Result<bool, AiError> {
        crate::structured_resources::read_text(
            self.db,
            resource,
            Some(&conversation.to_string()),
            false,
        )
        .map(|value| value.draft.is_some())
        .map_err(resource_error)
    }
}

fn resource_error(error: crate::structured_resources::Error) -> AiError {
    AiError::InvalidInput(error.to_string())
}
