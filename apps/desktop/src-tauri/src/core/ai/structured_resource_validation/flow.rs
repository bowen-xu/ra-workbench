//! Private control and safe causes for the existing synchronous validation chain.
use crate::{
    data_interface::plugin_resources::PluginDataError, runtime_plugins::native::SafeDatabaseError,
};
use std::convert::Infallible;

#[derive(Clone, Debug)]
pub(crate) struct DecisionOrigin {
    pub(crate) package_key: String,
    pub(crate) instance_id: String,
    pub(crate) native_request_id: String,
}

#[derive(Clone, Debug)]
pub(crate) enum FailureCause {
    Original,
    NativeRejection(Box<DecisionOrigin>),
    SourceData(Box<PluginDataError>),
    SourceDatabase(Box<SafeDatabaseError>),
    SourceReadUnavailable,
}

#[derive(Debug)]
pub(crate) struct Failure<E> {
    pub(crate) error: E,
    pub(crate) cause: FailureCause,
}
impl<E> Failure<E> {
    pub(crate) fn original(error: E) -> Self {
        Self {
            error,
            cause: FailureCause::Original,
        }
    }
    pub(crate) fn map_error<F>(self, map: impl FnOnce(E) -> F) -> Failure<F> {
        Failure {
            error: map(self.error),
            cause: self.cause,
        }
    }
    pub(crate) fn sql(error: E, phase: &'static str, cause: &rusqlite::Error) -> Self {
        Self {
            error,
            cause: FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(phase, cause))),
        }
    }
}

#[derive(Debug)]
pub(crate) enum Flow<E, P> {
    Fail(Failure<E>),
    Need(P),
}
impl<E, P> Flow<E, P> {
    pub(crate) fn original(error: E) -> Self {
        Self::Fail(Failure::original(error))
    }
    pub(crate) fn map_error<F>(self, map: impl FnOnce(E) -> F) -> Flow<F, P> {
        match self {
            Self::Fail(failure) => Flow::Fail(failure.map_error(map)),
            Self::Need(need) => Flow::Need(need),
        }
    }
}
impl<E> Flow<E, Infallible> {
    pub(crate) fn into_static(self) -> E {
        match self {
            Self::Fail(failure) => failure.error,
            Self::Need(never) => match never {},
        }
    }
}

impl<P> From<crate::ai::error::AiError> for Flow<crate::ai::error::AiError, P> {
    fn from(error: crate::ai::error::AiError) -> Self {
        let cause = match &error {
            crate::ai::error::AiError::Database(sql)
            | crate::ai::error::AiError::Knowledge(
                crate::ai::knowledge_contracts::KnowledgeError::Database(sql),
            ) => FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql("execution", sql))),
            _ => FailureCause::Original,
        };
        Self::Fail(Failure { error, cause })
    }
}
impl<P> From<rusqlite::Error> for Flow<crate::ai::error::AiError, P> {
    fn from(error: rusqlite::Error) -> Self {
        crate::ai::error::AiError::from(error).into()
    }
}
impl<P> From<crate::ai::knowledge_contracts::KnowledgeError>
    for Flow<crate::ai::error::AiError, P>
{
    fn from(error: crate::ai::knowledge_contracts::KnowledgeError) -> Self {
        crate::ai::error::AiError::from(error).into()
    }
}
