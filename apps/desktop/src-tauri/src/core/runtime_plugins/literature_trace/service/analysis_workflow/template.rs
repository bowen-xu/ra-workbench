//! Original lazy template reads, rechecked after every guard-free Native wait.
use super::*;
use crate::{
    ai::{
        candidate_contracts::{AiOverviewProfileContext, PaperTemplateRef},
        error::AiError,
        template_facts::TemplateFacts,
    },
    storage::AppStore,
};
use research_workbench_native_sdk::analysis_workflow::template::{
    TemplateDecisionV1, TemplateInputV1,
};
mod capture;
pub(super) use capture::{CapturedTemplate, TemplateResolution};

impl LiteratureTraceService {
    pub(crate) async fn resolve_analysis_template(
        &self,
        store: AppStore,
        key: &str,
        paper: &str,
        reference: &PaperTemplateRef,
    ) -> Result<AiOverviewProfileContext, AiError> {
        let mut resolution = TemplateResolution::default();
        // The original legacy path has three decisions, the resource path two.
        // One final capture checks every observation before returning its result.
        for _ in 0..4 {
            self.available().map_err(technical)?;
            let source = store.clone();
            let paper = paper.to_owned();
            let reference = reference.clone();
            let (next, captured) = tokio::task::spawn_blocking(move || {
                let db = source.connection()?;
                let captured = resolution.capture(&TemplateFacts::new(&db), &paper, &reference)?;
                Ok::<_, AiError>((resolution, captured))
            })
            .await
            .map_err(|_| {
                technical(
                    Self::error(
                        "plugin_host_source_worker_unsettled",
                        "阅读模板读取未确认完成。",
                    )
                    .package(key),
                )
            })??;
            resolution = next;
            match captured {
                CapturedTemplate::Resolved(profile) => return Ok(profile),
                CapturedTemplate::Need(input) => {
                    let (decision, origin) = self
                        .analysis_workflow_decision(
                            key,
                            WorkflowInputV1::Template {
                                input: Box::new(input.clone()),
                            },
                        )
                        .await
                        .map_err(technical)?;
                    let WorkflowDecisionV1::Template { decision } = decision else {
                        return Err(identified(
                            "plugin_host_response_mismatch",
                            "阅读模板结果种类不匹配。",
                            &origin,
                        ));
                    };
                    resolution.record(input, decision, origin);
                }
            }
        }
        Err(technical(
            Self::error(
                "plugin_host_response_mismatch",
                "阅读模板决定未在原有限步骤内完成。",
            )
            .package(key),
        ))
    }
}

pub(in super::super) fn technical(failure: Box<NativeHostError>) -> AiError {
    AiError::NativeWorkflow {
        code: "native_workflow_failed",
        failure,
    }
}

pub(in super::super) fn identified(code: &str, message: &str, origin: &DecisionOrigin) -> AiError {
    let mut failure = LiteratureTraceService::error(code, message).package(&origin.package_key);
    failure.instance_id = Some(origin.instance_id.clone());
    failure.request_id = Some(origin.native_request_id.clone());
    technical(failure)
}

pub(in super::super) fn rejected(error: impl Into<AiError>, origin: &DecisionOrigin) -> AiError {
    let error = error.into();
    let mut failure =
        LiteratureTraceService::error(error.code(), error.to_string()).package(&origin.package_key);
    failure.instance_id = Some(origin.instance_id.clone());
    failure.request_id = Some(origin.native_request_id.clone());
    AiError::NativeWorkflow {
        code: error.code(),
        failure,
    }
}
