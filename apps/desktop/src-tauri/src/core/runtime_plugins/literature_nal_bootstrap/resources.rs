use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

mod definitions;

const T: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const N: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const I: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};

#[derive(Clone)]
pub(crate) struct BootstrapResources {
    pub(super) legacy_header: AcceptedDataResource,
    pub(super) legacy_collections: AcceptedDataResource,
    pub(super) legacy_types: AcceptedDataResource,
    pub(super) legacy_related_used: AcceptedDataResource,
    pub(super) legacy_relations: AcceptedDataResource,
    pub(super) legacy_memberships: AcceptedDataResource,
    pub(super) legacy_assignments: AcceptedDataResource,
    pub(super) legacy_import_flag: AcceptedDataResource,
    pub(super) repair_version: AcceptedDataResource,
    pub(super) repair_records: AcceptedDataResource,
    pub(super) repair_relation_kind: AcceptedDataResource,
    pub(super) repair_statement: AcceptedDataResource,
    pub(super) repair_flag: AcceptedDataResource,
    pub(super) cleanup_exists: AcceptedDataResource,
    pub(super) state_revision: AcceptedDataResource,
    pub(super) state_concepts: AcceptedDataResource,
    pub(super) state_statements: AcceptedDataResource,
    pub(super) state_annotations: AcceptedDataResource,
    pub(super) state_rules: AcceptedDataResource,
    pub(super) statement_relation_kind: AcceptedDataResource,
    pub(super) statement_insert: AcceptedDataResource,
    pub(super) concept_delete: AcceptedDataResource,
    pub(super) concept_search_delete: AcceptedDataResource,
    pub(super) concept_name_exists: AcceptedDataResource,
    pub(super) term_read: AcceptedDataResource,
    pub(super) term_lookup: AcceptedDataResource,
    pub(super) term_insert: AcceptedDataResource,
    pub(super) concept_main: AcceptedDataResource,
    pub(super) concept_terms: AcceptedDataResource,
    pub(super) concept_insert: AcceptedDataResource,
    pub(super) concept_preferred_insert: AcceptedDataResource,
    pub(super) concept_name_available: AcceptedDataResource,
    pub(super) research_revision_insert: AcceptedDataResource,
    pub(super) representation_lookup: AcceptedDataResource,
    pub(super) representation_insert: AcceptedDataResource,
    pub(super) semantic_old_jobs_cancel: AcceptedDataResource,
    pub(super) semantic_old_records_tombstone: AcceptedDataResource,
    pub(super) embedding_space_insert: AcceptedDataResource,
    pub(super) generation_next: AcceptedDataResource,
    pub(super) generation_insert: AcceptedDataResource,
    pub(super) embedding_job_lookup: AcceptedDataResource,
    pub(super) embedding_record_insert: AcceptedDataResource,
    pub(super) embedding_job_insert: AcceptedDataResource,
    pub(super) generation_active: AcceptedDataResource,
    pub(super) active_name_keys: AcceptedDataResource,
}

impl BootstrapResources {
    pub(crate) fn state_resources(&self) -> super::NalLoadResources<'_> {
        super::NalLoadResources {
            revision: &self.state_revision,
            concepts: &self.state_concepts,
            statements: &self.state_statements,
            annotations: &self.state_annotations,
            rules: &self.state_rules,
        }
    }

    pub(crate) fn mutation_concept_delete(&self) -> &AcceptedDataResource {
        &self.concept_delete
    }

    pub(crate) fn mutation_concept_search_delete(&self) -> &AcceptedDataResource {
        &self.concept_search_delete
    }

    pub(crate) fn mutation_statement_insert(&self) -> &AcceptedDataResource {
        &self.statement_insert
    }

    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        definitions::load(&Loader { catalog, package })
    }
}

struct Loader<'a> {
    catalog: &'a RuntimePluginCatalog,
    package: &'a RuntimePluginPackage,
}
impl Loader<'_> {
    fn load(
        &self,
        name: &str,
        sha256: &str,
        access: DataAccess,
        parameters: &[DataField],
        columns: &[DataField],
    ) -> Result<AcceptedDataResource, RuntimePluginError> {
        AcceptedDataResource::load(
            self.catalog,
            self.package,
            "literature_library",
            &format!("data/nal-bootstrap-{name}.v1.sql"),
            DataResourceGrant {
                id: format!("literature.nal-bootstrap.{name}"),
                version: 1,
                sha256: sha256.into(),
                access,
                parameters: parameters.to_vec(),
                columns: columns.to_vec(),
            },
        )
    }
}
