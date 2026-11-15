//! Select domain policy while retaining the existing shared Agent and Core grants.
use super::*;
use crate::{
    ai::{
        generation_hooks::{PaperCompletionFuture, PaperTaskPrompt, PreparedPaperWorkflow},
        structured_generation_support::{self as facts, StructuredGenerationFacts},
        structured_resource_validation::DecisionOrigin,
    },
    runtime_plugins::{literature_trace::binding::NoteTraceBinding, native::NativeHostError},
};
use research_agent_core::{submission::ContextText, user_input::UserInput};
use research_workbench_native_sdk::{
    analysis_workflow::{
        AnalysisWorkflowError,
        prompt::{PromptSourceV1, TaskPromptInputV1},
        submission::{CompletionProjectionV1, SubmissionScopeV1},
        wire::{WorkflowDecisionV1, WorkflowInputV1},
    },
    candidate_contracts::AiOverviewProfileContext,
    paper_context_wire::encode_payload,
};

pub(super) async fn prepare(
    store: &AppStore,
    conversation: Uuid,
    request: Uuid,
    frozen: Option<&SubmissionInput>,
) -> Result<Option<Box<dyn PreparedPaperWorkflow>>, AiError> {
    let Some(binding) = store.native_note_trace().cloned() else {
        let store = store.clone();
        let frozen = frozen.cloned();
        return tokio::task::spawn_blocking(move || {
            (store.require_generation_policy_source()?.prepare_submission)(
                &store,
                conversation,
                request,
                frozen.as_ref(),
            )
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?;
    };
    let capture_store = store.clone();
    let selection =
        tokio::task::spawn_blocking(move || capture(&capture_store, conversation, request))
            .await
            .map_err(|_| AiError::TemporaryUnavailable)??;
    let Some(selection) = selection else {
        return Ok(None);
    };
    let (decision, origin) = binding
        .service
        .analysis_workflow_decision(
            &binding.package_key,
            WorkflowInputV1::SubmissionScope {
                task_kind: selection.task_kind.clone(),
                paper_ids: selection.papers.clone(),
                attachment_ids: selection.attachments.clone(),
            },
        )
        .await
        .map_err(technical)?;
    let WorkflowDecisionV1::SubmissionScope { result } = decision else {
        return Err(mismatch(&origin));
    };
    let scope = result.map_err(|error| rejected(error, &origin))?;
    let prepare_store = store.clone();
    let frozen = frozen.cloned();
    let prepare_scope = scope.clone();
    let (target, template, grants, prompt) = tokio::task::spawn_blocking(move || {
        let store = &prepare_store;
        let scope = prepare_scope;
        let db = store.connection()?;
        let facts = StructuredGenerationFacts::new(&db, store);
        if capture_in(&facts, conversation, request)?.as_ref() != Some(&selection) {
            return Err(changed(&origin));
        }
        let grants = match frozen.as_ref() {
            Some(input) => input.context.clone(),
            None => facts.grants(
                &scope.paper_id,
                &scope.attachment_ids,
                conversation,
                &scope.kind,
                &selection.bundle_id,
            )?,
        };
        let target = facts.target(&scope.paper_id, &scope.kind)?;
        let template = if scope.kind == "portrait" {
            facts.writable_template(&scope.paper_id, &grants)?
        } else {
            None
        };
        let prompt = match frozen.as_ref() {
            Some(input) => Prompt::Frozen(PaperTaskPrompt {
                text: input.text.clone(),
                user_input: input.user_input.clone(),
            }),
            None => {
                facts::require_writable_target(&grants, &target.id)?;
                let (task_template, reading_contract) = facts.prompt_templates()?;
                Prompt::Render(Box::new(TaskPromptInputV1 {
                    task_template,
                    reading_contract,
                    sources: grants
                        .iter()
                        .filter_map(|source| {
                            facts::prompt_resource(source).map(|resource| PromptSourceV1 {
                                source: source.clone(),
                                resource,
                            })
                        })
                        .collect(),
                    kind: scope.kind.clone(),
                    profile: selection.profile.clone(),
                    preferences: selection.goal.clone(),
                }))
            }
        };
        Ok((target.id, template, grants, prompt))
    })
    .await
    .map_err(|_| AiError::TemporaryUnavailable)??;
    let prompt = match prompt {
        Prompt::Frozen(prompt) => prompt,
        Prompt::Render(input) => {
            let (result, origin) = binding
                .service
                .render_analysis_task_prompt(&binding.package_key, *input)
                .await
                .map_err(technical)?;
            result.map_err(|error| rejected(error, &origin))?
        }
    };
    Ok(Some(Box::new(Prepared {
        binding,
        scope,
        target,
        template,
        grants,
        prompt,
    })))
}

enum Prompt {
    Frozen(PaperTaskPrompt),
    Render(Box<TaskPromptInputV1>),
}

#[derive(PartialEq)]
struct Selection {
    run_id: String,
    bundle_id: String,
    task_kind: String,
    papers: Vec<String>,
    attachments: Vec<String>,
    profile: Option<AiOverviewProfileContext>,
    goal: String,
}

fn capture(
    store: &AppStore,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<Selection>, AiError> {
    let db = store.connection()?;
    capture_in(
        &StructuredGenerationFacts::new(&db, store),
        conversation,
        request,
    )
}

fn capture_in(
    facts: &StructuredGenerationFacts<'_>,
    conversation: Uuid,
    request: Uuid,
) -> Result<Option<Selection>, AiError> {
    if !facts.present()? {
        return Ok(None);
    }
    let Some(run) = facts.run(
        conversation,
        request,
        ["paper_overview", "paper_profile_classification"],
    )?
    else {
        return Ok(None);
    };
    let (bundle_id, bundle) = facts.bundle(&run.id)?;
    Ok(Some(Selection {
        run_id: run.id,
        bundle_id,
        task_kind: run.task_kind,
        papers: bundle.items.into_iter().map(|item| item.paper_id).collect(),
        attachments: run
            .scope_items
            .into_iter()
            .filter(|item| item.item_kind == "attachment")
            .map(|item| item.object_id)
            .collect(),
        profile: run.overview_profile,
        goal: run.user_goal,
    }))
}

struct Prepared {
    binding: NoteTraceBinding,
    scope: SubmissionScopeV1,
    target: String,
    template: Option<(String, String)>,
    grants: Vec<ContextText>,
    prompt: PaperTaskPrompt,
}

impl PreparedPaperWorkflow for Prepared {
    fn kind(&self) -> &str {
        &self.scope.kind
    }
    fn grants(&self) -> &[ContextText] {
        &self.grants
    }
    fn prompt_text(&self) -> &str {
        &self.prompt.text
    }
    fn prompt_user_input(&self) -> &[UserInput] {
        &self.prompt.user_input
    }
    fn decorate(
        &self,
        input: &mut ProviderInput,
        submission: &SubmissionInput,
    ) -> Result<(), AiError> {
        facts::decorate_submission(input, submission, &self.scope.system_text)
    }
    fn complete<'a>(
        &'a self,
        store: &'a AppStore,
        conversation: Uuid,
        request: Uuid,
        mut response: VisionGenerationResponse,
    ) -> PaperCompletionFuture<'a> {
        Box::pin(async move {
            let captured = self.receipt(store, conversation, request).await?;
            let encoded = captured.clone();
            let input_cbor_base64 = tokio::task::spawn_blocking(move || encode_payload(&encoded))
                .await
                .map_err(|_| AiError::TemporaryUnavailable)?
                .map_err(|_| AiError::TemporaryUnavailable)?;
            let (decision, origin) = self
                .binding
                .service
                .analysis_workflow_decision(
                    &self.binding.package_key,
                    WorkflowInputV1::CompletionProjection { input_cbor_base64 },
                )
                .await
                .map_err(technical)?;
            let WorkflowDecisionV1::CompletionProjection { result } = decision else {
                return Err(mismatch(&origin));
            };
            response.content = result.map_err(|error| rejected(error, &origin))?;
            if self.receipt(store, conversation, request).await? != captured {
                return Err(changed(&origin));
            }
            Ok(response)
        })
    }
}

impl Prepared {
    async fn receipt(
        &self,
        store: &AppStore,
        conversation: Uuid,
        request: Uuid,
    ) -> Result<CompletionProjectionV1, AiError> {
        let store = store.clone();
        let target = self.target.clone();
        let template = self.template.clone();
        let kind = self.scope.kind.clone();
        tokio::task::spawn_blocking(move || {
            let db = store.connection()?;
            let (content, template) = StructuredGenerationFacts::new(&db, &store)
                .completion_receipt(
                    &target,
                    template.as_ref().map(|(id, _)| id.as_str()),
                    conversation,
                    request,
                )?;
            Ok(CompletionProjectionV1 {
                kind,
                content,
                template,
            })
        })
        .await
        .map_err(|_| AiError::TemporaryUnavailable)?
    }
}

fn technical(failure: Box<NativeHostError>) -> AiError {
    AiError::NativeWorkflow {
        code: "native_workflow_failed",
        failure,
    }
}

fn rejected(error: AnalysisWorkflowError, origin: &DecisionOrigin) -> AiError {
    let error = AiError::from(error);
    AiError::NativeWorkflow {
        code: error.code(),
        failure: identified(error.code(), error.to_string(), origin),
    }
}

fn mismatch(origin: &DecisionOrigin) -> AiError {
    technical(identified(
        "plugin_host_response_mismatch",
        "论文共享任务的结果种类不匹配。".into(),
        origin,
    ))
}

fn changed(origin: &DecisionOrigin) -> AiError {
    technical(identified(
        "plugin_host_source_changed",
        "论文共享任务的来源已变化，请重新准备。".into(),
        origin,
    ))
}

fn identified(code: &str, message: String, origin: &DecisionOrigin) -> Box<NativeHostError> {
    let mut error = NativeHostError::for_profile(
        crate::runtime_plugins::native::NativeProfile::NoteTraceDecision,
        code,
        message,
    )
    .package(&origin.package_key);
    error.instance_id = Some(origin.instance_id.clone());
    error.request_id = Some(origin.native_request_id.clone());
    error
}
