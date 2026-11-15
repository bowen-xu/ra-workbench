//! Native wire adapters; provider-specific objects never enter the shared core.
#[path = "chat_decoder.rs"]
pub mod chat_decoder;
#[path = "chat_policy.rs"]
mod chat_policy;
#[path = "chat_request.rs"]
pub(crate) mod chat_request;
#[path = "chat_transport.rs"]
pub mod chat_transport;
#[path = "selection.rs"]
mod selection;
#[path = "sse.rs"]
pub(crate) mod sse;
#[path = "transport_error.rs"]
mod transport_error;
pub use crate::ai::agent_configuration::{
    AgentHost, AgentModelPreparationError, AgentModelResolver, AgentModelSelection,
    PreparedAgentModel,
};
pub use chat_policy::ChatRequestPolicy;
pub use chat_request::OutputTokenParameter;
pub use selection::ModelSelection;
#[path = "responses_accounting.rs"]
pub(crate) mod responses_accounting;
#[path = "responses_chat_compatibility.rs"]
mod responses_chat_compatibility;
#[path = "responses_continuation.rs"]
pub mod responses_continuation;
#[path = "responses_decoder.rs"]
pub mod responses_decoder;
#[path = "responses_metadata.rs"]
pub mod responses_metadata;
#[path = "responses_native.rs"]
pub(crate) mod responses_native;
#[path = "responses_reasoning.rs"]
pub mod responses_reasoning;
#[path = "responses_request.rs"]
pub mod responses_request;
#[path = "responses_transport.rs"]
pub mod responses_transport;
