//! Native role declaration frozen with the existing context policy, not renderer IPC.
use super::*;
impl AgentExecutionProfile {
    pub fn with_chat_developer_role(
        mut self,
        model: &ModelIdentity,
        enabled: bool,
    ) -> Result<Self, UnsupportedExecutionProfile> {
        if model != &self.model {
            return Err(UnsupportedExecutionProfile);
        }
        self.chat_developer_model = enabled.then(|| model.clone());
        Ok(self)
    }
    pub(super) fn chat_transport(
        &self,
        prepared: PreparedAgentModel,
        declared: Option<&ModelIdentity>,
    ) -> Result<super::super::model::chat_transport::ChatTransport, AgentModelPreparationError>
    {
        if declared.is_some_and(|model| model != &self.model) {
            return Err(research_agent_core::model::ModelError::UnsupportedCapability.into());
        }
        let transport = prepared.into_chat_transport_with_policy(
            self.capabilities.clone(),
            self.token_parameter,
            self.request_policy,
        )?;
        match declared {
            Some(model) => transport
                .with_developer_role(model.clone())
                .map_err(Into::into),
            None => Ok(transport),
        }
    }
}
#[cfg(test)]
#[path = "chat_role/tests.rs"]
mod tests;
