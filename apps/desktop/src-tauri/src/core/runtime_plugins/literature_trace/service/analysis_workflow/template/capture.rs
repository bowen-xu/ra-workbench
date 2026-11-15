//! Core observations stay owned and every earlier decision binds the same facts.
use super::*;

struct Observation {
    input: TemplateInputV1,
    decision: TemplateDecisionV1,
    origin: DecisionOrigin,
}
#[derive(Default)]
pub(in super::super) struct TemplateResolution {
    observations: Vec<Observation>,
}
pub(in super::super) enum CapturedTemplate {
    Need(TemplateInputV1),
    Resolved(AiOverviewProfileContext),
}

impl TemplateResolution {
    pub(in super::super) fn record(
        &mut self,
        input: TemplateInputV1,
        decision: TemplateDecisionV1,
        origin: DecisionOrigin,
    ) {
        self.observations.push(Observation {
            input,
            decision,
            origin,
        });
    }

    pub(in super::super) fn capture(
        &self,
        facts: &TemplateFacts<'_>,
        paper: &str,
        reference: &PaperTemplateRef,
    ) -> Result<CapturedTemplate, AiError> {
        if matches!(reference, PaperTemplateRef::Resource { .. }) {
            let (selection_revision, binding) = facts.reading_template(paper)?;
            let input = TemplateInputV1::ResourceBinding {
                reference: reference.clone(),
                selection_revision,
                binding,
            };
            if !self.verified(0, &input)? {
                return Ok(CapturedTemplate::Need(input));
            }
            let input = TemplateInputV1::ResourceContent {
                paper_id: paper.into(),
                reference: reference.clone(),
                selection_revision,
                raw: facts.resource_revision(reference)?,
            };
            return self.resolved(1, input);
        }
        let input = TemplateInputV1::LegacyAvailability {
            managed_resources: facts.managed_resources()?,
        };
        if !self.verified(0, &input)? {
            return Ok(CapturedTemplate::Need(input));
        }
        let input = TemplateInputV1::LegacyIdentity {
            reference: reference.clone(),
            latest: facts.latest_portrait(paper, "paper_portrait_v1")?,
        };
        if !self.verified(1, &input)? {
            return Ok(CapturedTemplate::Need(input));
        }
        self.resolved(2, facts.legacy_content(paper, reference)?)
    }

    fn observed(
        &self,
        index: usize,
        input: &TemplateInputV1,
    ) -> Result<Option<&Observation>, AiError> {
        let Some(observation) = self.observations.get(index) else {
            return Ok(None);
        };
        if observation.input != *input {
            return Err(identified(
                "plugin_host_source_changed",
                "阅读模板来源已变化，请重新准备。",
                &observation.origin,
            ));
        }
        if let TemplateDecisionV1::Rejected { error } = &observation.decision {
            return Err(rejected(error.clone(), &observation.origin));
        }
        Ok(Some(observation))
    }

    fn verified(&self, index: usize, input: &TemplateInputV1) -> Result<bool, AiError> {
        let Some(observation) = self.observed(index, input)? else {
            return Ok(false);
        };
        if !matches!(observation.decision, TemplateDecisionV1::Verified {}) {
            return Err(identified(
                "plugin_host_response_mismatch",
                "阅读模板校验结果种类不匹配。",
                &observation.origin,
            ));
        }
        Ok(true)
    }

    fn resolved(&self, index: usize, input: TemplateInputV1) -> Result<CapturedTemplate, AiError> {
        let Some(observation) = self.observed(index, &input)? else {
            return Ok(CapturedTemplate::Need(input));
        };
        match &observation.decision {
            TemplateDecisionV1::Resolved { profile } => {
                Ok(CapturedTemplate::Resolved(profile.clone()))
            }
            _ => Err(identified(
                "plugin_host_response_mismatch",
                "阅读模板解析结果种类不匹配。",
                &observation.origin,
            )),
        }
    }
}
