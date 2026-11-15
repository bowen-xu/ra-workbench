//! Exact-model native DI for hook context; byte ceilings are not token claims.
use super::*;
use crate::agent::compaction_hook_spill::HookContextBudget;

impl AgentExecutionProfile {
    pub fn with_hook_context_budget(
        mut self,
        model: &ModelIdentity,
        inline_bytes: usize,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        self.validate_hook_budget(model, inline_bytes)?;
        self.hook_context_bytes = inline_bytes;
        Ok(self)
    }
    fn validate_hook_budget(
        &self,
        model: &ModelIdentity,
        bytes: usize,
    ) -> Result<(), UnsupportedExecutionProfile> {
        if &self.model != model
            || bytes > self.max_input_bytes
            || bytes > self.limits.tool_output_bytes
            || HookContextBudget::new(model.clone(), bytes).is_err()
        {
            return Err(UnsupportedExecutionProfile);
        }
        Ok(())
    }
    pub(crate) fn hook_context_budget(
        &self,
        model: &ModelIdentity,
    ) -> Result<HookContextBudget, UnsupportedExecutionProfile> {
        self.validate_hook_budget(model, self.hook_context_bytes)?;
        HookContextBudget::new(model.clone(), self.hook_context_bytes)
            .map_err(|_| UnsupportedExecutionProfile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hook_budget_is_exact_model_native_policy_and_respects_profile_ceilings() {
        let model = ModelIdentity {
            provider_id: "deepseek".into(),
            model_id: "deepseek-v4-flash".into(),
            revision: Some("r1".into()),
        };
        let profile = execution_profile(&model, AgentExecutionMode::DeepSeekNonThinking).unwrap();
        assert_eq!(profile.hook_context_bytes, 10_000);
        assert!(profile.hook_context_budget(&model).is_ok());
        let mut other = model.clone();
        other.revision = Some("r2".into());
        assert!(profile.hook_context_budget(&other).is_err());
        assert!(
            profile
                .clone()
                .with_hook_context_budget(&other, 512)
                .is_err()
        );
        for invalid in [
            0,
            255,
            profile.limits.tool_output_bytes + 1,
            1024 * 1024 + 1,
        ] {
            assert!(
                profile
                    .clone()
                    .with_hook_context_budget(&model, invalid)
                    .is_err()
            );
        }
        let changed = profile
            .clone()
            .with_hook_context_budget(&model, 512)
            .unwrap();
        assert_eq!(changed.hook_context_bytes, 512);
        assert_eq!(profile.hook_context_bytes, 10_000);
    }
}
