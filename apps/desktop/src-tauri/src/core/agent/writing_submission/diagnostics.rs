//! Safe source provenance shared by the existing writing preparation errors.
use serde::Serialize;
use uuid::Uuid;
#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum WritingSourceDiagnostic {
    Native {
        package_key: String,
        instance_id: String,
        request_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        rejection: Option<super::WritingContextError>,
    },
    NativeFailure {
        error: super::WritingNativeError,
    },
    Data {
        error: Box<crate::data_interface::plugin_resources::PluginDataError>,
    },
    Database {
        error: Box<crate::runtime_plugins::native::SafeDatabaseError>,
    },
    Unavailable,
}
pub(crate) fn source_diagnostic(
    cause: crate::ai::structured_resource_validation::FailureCause,
    rejection: Option<super::WritingContextError>,
) -> Option<WritingSourceDiagnostic> {
    use crate::ai::structured_resource_validation::FailureCause;
    match cause {
        FailureCause::Original => None,
        FailureCause::NativeRejection(origin) => Some(WritingSourceDiagnostic::Native {
            package_key: origin.package_key,
            instance_id: origin.instance_id,
            request_id: origin.native_request_id,
            rejection,
        }),
        FailureCause::SourceData(error) => Some(WritingSourceDiagnostic::Data { error }),
        FailureCause::SourceDatabase(error) => Some(WritingSourceDiagnostic::Database { error }),
        FailureCause::SourceReadUnavailable => Some(WritingSourceDiagnostic::Unavailable),
    }
}

/// Adds provenance without changing the original code/reason classification.
#[derive(Debug)]
pub struct WritingFailure<E> {
    error: E,
    writing_source: WritingPreparationDiagnostic,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WritingPreparationDiagnostic {
    conversation_id: Uuid,
    manuscript_id: Uuid,
    operation_id: Uuid,
    input_id: Uuid,
    source: WritingSourceDiagnostic,
}
impl<E: Serialize> Serialize for WritingFailure<E> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Use the concrete JSON serializer at this recursive enum boundary.
        // A derived flatten would grow its serializer type at each nesting.
        let mut value = serde_json::to_value(&self.error).map_err(serde::ser::Error::custom)?;
        value
            .as_object_mut()
            .expect("queued preparation errors are objects")
            .insert(
                "writingSource".into(),
                serde_json::to_value(&self.writing_source).map_err(serde::ser::Error::custom)?,
            );
        value.serialize(serializer)
    }
}
impl<E> WritingFailure<E> {
    pub(crate) fn new(
        error: E,
        source: WritingSourceDiagnostic,
        conversation_id: Uuid,
        manuscript_id: Uuid,
        operation_id: Uuid,
        input_id: Uuid,
    ) -> Self {
        Self {
            error,
            writing_source: WritingPreparationDiagnostic {
                conversation_id,
                manuscript_id,
                operation_id,
                input_id,
                source,
            },
        }
    }
}
