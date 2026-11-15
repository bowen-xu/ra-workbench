use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const OPTIONAL_TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};

#[derive(Clone)]
pub(crate) struct DoiResources {
    package_key: String,
    pub(super) result_failed: AcceptedDataResource,
    pub(super) candidate_insert: AcceptedDataResource,
    pub(super) job_queued: AcceptedDataResource,
}

impl DoiResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        Ok(Self {
            package_key: package.package_key.clone(),
            result_failed: load(
                catalog,
                package,
                "result-failed",
                "8e361f52e94765b86e2f7eb123f9712cbfbab1078dc58b35145218a9c97d2f9b",
                &[TEXT, TEXT],
            )?,
            candidate_insert: load(
                catalog,
                package,
                "candidate-insert",
                "6a1a5cbc42a5e39de780786d4d95ff43c97c0a8fd71ccf3c15beeee358e294dc",
                &[TEXT, OPTIONAL_TEXT, TEXT, TEXT, TEXT, TEXT, TEXT],
            )?,
            job_queued: load(
                catalog,
                package,
                "job-queued",
                "e19da19f93cf50ad23c2134a6a1864ba0ce7783d462d99497c655d1ffac83cfb",
                &[TEXT],
            )?,
        })
    }

    pub(crate) fn package_key(&self) -> &str {
        &self.package_key
    }
}

fn load(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
    suffix: &str,
    sha256: &str,
    parameters: &[DataField],
) -> Result<AcceptedDataResource, RuntimePluginError> {
    AcceptedDataResource::load(
        catalog,
        package,
        "literature_library",
        &format!("data/doi-preview-{suffix}.v1.sql"),
        DataResourceGrant {
            id: format!("literature.doi-preview.{suffix}"),
            version: 1,
            sha256: sha256.into(),
            access: DataAccess::Write,
            parameters: parameters.to_vec(),
            columns: Vec::new(),
        },
    )
}
