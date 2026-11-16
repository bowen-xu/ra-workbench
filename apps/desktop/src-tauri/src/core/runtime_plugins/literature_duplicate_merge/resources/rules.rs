//! Explicit fixed grants only; rule SQL remains in the accepted plugin package.
use crate::runtime_plugins::{
    RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, data::AcceptedDataResource,
};
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};

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
const NI: DataField = DataField {
    data_type: DataType::Integer,
    nullable: true,
};

#[derive(Clone)]
pub(crate) struct MergeRuleResources {
    pub(crate) legacy_membership_seed: AcceptedDataResource,
    pub(crate) legacy_tagged_seed: AcceptedDataResource,
    pub(crate) legacy_collection_seed: AcceptedDataResource,
    pub(crate) legacy_linked_seed: AcceptedDataResource,
    pub(crate) legacy_type_probe_required: AcceptedDataResource,
    pub(crate) legacy_type_seed: AcceptedDataResource,
    pub(crate) legacy_inheritance_seed: AcceptedDataResource,
    pub(crate) legacy_repair_seed: AcceptedDataResource,
    pub(crate) legacy_linked_statement: AcceptedDataResource,
    pub(crate) legacy_membership_statement: AcceptedDataResource,
    pub(crate) legacy_assignment_statement: AcceptedDataResource,
    pub(crate) legacy_relation_forward_statement: AcceptedDataResource,
    pub(crate) legacy_relation_reverse_statement: AcceptedDataResource,
    pub(crate) legacy_preferred_term_id: AcceptedDataResource,
    pub(crate) legacy_term_input: AcceptedDataResource,
    pub(crate) legacy_name_resolve: AcceptedDataResource,
    pub(crate) legacy_concept_details: AcceptedDataResource,
    pub(crate) legacy_render_concept: AcceptedDataResource,
    pub(crate) legacy_repair_wrap: AcceptedDataResource,
    pub(crate) legacy_cleanup_select: AcceptedDataResource,
    pub(crate) legacy_replace_paper: AcceptedDataResource,
    pub(crate) semantic_space_policy: AcceptedDataResource,
    pub(crate) semantic_generation_path: AcceptedDataResource,
    pub(crate) semantic_record_row_id: AcceptedDataResource,
    pub(crate) semantic_job_idempotency: AcceptedDataResource,
    pub(crate) legacy_term_metadata: AcceptedDataResource,
    pub(crate) legacy_concept_metadata: AcceptedDataResource,
    pub(crate) legacy_repair_skip: AcceptedDataResource,
    pub(crate) legacy_concept_delete_error: AcceptedDataResource,
    pub(crate) legacy_nal_insert_policy: AcceptedDataResource,
}

impl MergeRuleResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |suffix: &str,
                    id: &str,
                    hash: &str,
                    parameters: &[DataField],
                    columns: &[DataField]| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/duplicate-merge-{suffix}.v1.sql"),
                DataResourceGrant {
                    id: id.into(),
                    version: 1,
                    sha256: hash.into(),
                    access: DataAccess::Read,
                    parameters: parameters.to_vec(),
                    columns: columns.to_vec(),
                },
            )
        };
        Ok(Self {
            legacy_membership_seed: load(
                "legacy-membership-seed",
                "literature.duplicate-merge.legacy.membership-seed",
                "db67a3598d31c881c5b2cc7e360f48dc3815db905d75653564b03fa34b52e37a",
                &[I],
                &[T, T],
            )?,
            legacy_tagged_seed: load(
                "legacy-tagged-seed",
                "literature.duplicate-merge.legacy.tagged-seed",
                "4cc58fa31cc7757271cac6ff055048407b52b4bd87f0114e017d3e28656306e2",
                &[I],
                &[T, T],
            )?,
            legacy_collection_seed: load(
                "legacy-collection-seed",
                "literature.duplicate-merge.legacy.collection-seed",
                "fbf8532d0f2764c148e9c38ec4b1970c6076f1c29b17cfdb5bc4afbe5f56d34f",
                &[T, T],
                &[T, T],
            )?,
            legacy_linked_seed: load(
                "legacy-linked-seed",
                "literature.duplicate-merge.legacy.linked-seed",
                "9b31e03c470e112273afb9dc80246f4162185f728e80d43102aacda14fc20586",
                &[I],
                &[T, T],
            )?,
            legacy_type_probe_required: load(
                "legacy-type-probe-required",
                "literature.duplicate-merge.legacy.type-probe-required",
                "30e3946c2e09bf9694d6ef98dfb30ac5cfa9e1b986c0946760aba84249cacf80",
                &[T],
                &[I],
            )?,
            legacy_type_seed: load(
                "legacy-type-seed",
                "literature.duplicate-merge.legacy.type-seed",
                "e9fd54739d0ca7b0ce44eba9fd920040035f25093f58acd7358e410d78d98a6b",
                &[T, T, T, I, NI],
                &[T, T],
            )?,
            legacy_inheritance_seed: load(
                "legacy-inheritance-seed",
                "literature.duplicate-merge.legacy.inheritance-seed",
                "d744395f455f9c942b5823715debcacb7dc94dd58fa20f3613efc7cc45ad1e71",
                &[T],
                &[T, T],
            )?,
            legacy_repair_seed: load(
                "legacy-repair-seed",
                "literature.duplicate-merge.legacy.repair-seed",
                "6b97dda7ae3abf405807ce4b78866b1453bb5d0bb993c0e3b6308586ebfa1ebf",
                &[I],
                &[T, T],
            )?,
            legacy_linked_statement: load(
                "legacy-linked-statement",
                "literature.duplicate-merge.legacy.linked-statement",
                "3f441c4da6e188617ee75a536f81c1e4d8cffe16072cf47c2ca2dcc0d3eabe19",
                &[T, T, T],
                &[T, T, T, T],
            )?,
            legacy_membership_statement: load(
                "legacy-membership-statement",
                "literature.duplicate-merge.legacy.membership-statement",
                "06688429b11ef0f63637e4d7868c7468b33a93d4a5390ec1fc80e07f961ad089",
                &[T, T, T],
                &[T, T, T, T],
            )?,
            legacy_assignment_statement: load(
                "legacy-assignment-statement",
                "literature.duplicate-merge.legacy.assignment-statement",
                "e8431dd83cd3eef12c44a6d982ec2f3d3d185a03636943112b853a91894d2894",
                &[T, T, T, T],
                &[T, T, T, T],
            )?,
            legacy_relation_forward_statement: load(
                "legacy-relation-forward-statement",
                "literature.duplicate-merge.legacy.relation-forward-statement",
                "801367809cae0a83f46c21299019465bdf6b592f8c30e7d4259bf51388edaf31",
                &[T, T, T, N, N],
                &[T, T, T, T],
            )?,
            legacy_relation_reverse_statement: load(
                "legacy-relation-reverse-statement",
                "literature.duplicate-merge.legacy.relation-reverse-statement",
                "6954a3cf66f447126fdc1050798f971e49b5956974a0e028de9bfe09d0908fe8",
                &[T, T, T, N, NI],
                &[T, T, T, T],
            )?,
            legacy_preferred_term_id: load(
                "legacy-preferred-term-id",
                "literature.duplicate-merge.legacy.preferred-term-id",
                "7752ff3e312dd808fb760bf61f1544912727667ab1295a89cb95aae639b3c9b1",
                &[T],
                &[T],
            )?,
            legacy_term_input: load(
                "legacy-term-input",
                "literature.duplicate-merge.legacy.term-input",
                "9a702b9ad1fcaf67768bf4d4c7a58dda48e93ec2e1110808c2ec9d34e38243b6",
                &[T],
                &[N, N, T, T, T],
            )?,
            legacy_name_resolve: load(
                "legacy-name-resolve",
                "literature.duplicate-merge.legacy.name-resolve",
                "e831e303b5fa84cb9cd4baaf4bfe4c0b2d8b248c34464f9c91d752cdf4f1c78c",
                &[T],
                &[T],
            )?,
            legacy_concept_details: load(
                "legacy-concept-details",
                "literature.duplicate-merge.legacy.concept-details",
                "b72c96b53cceea2e452dbbd82873f3ac235b58367f0593b3e1aa67a22f39d54d",
                &[T, T],
                &[N, N, T, T, T, T, T, T],
            )?,
            legacy_render_concept: load(
                "legacy-render-concept",
                "literature.duplicate-merge.legacy.render-concept",
                "f0206390c401766642769a9ce2b9920dd05531e2a080492aca38ce24277b626e",
                &[T],
                &[T, T, T],
            )?,
            legacy_repair_wrap: load(
                "legacy-repair-wrap",
                "literature.duplicate-merge.legacy.repair-wrap",
                "bd05ef03634125556bd23bb0c17565da2caa9c409c2f8511a7549a68a6d59ca8",
                &[T, T, T],
                &[T, T],
            )?,
            legacy_cleanup_select: load(
                "legacy-cleanup-select",
                "literature.duplicate-merge.legacy.cleanup-select",
                "58c6c037207288243e4c4c049b1b1e98f2913a84d7c2944e4b830558adc2ec41",
                &[T, T, T, T],
                &[T],
            )?,
            legacy_replace_paper: load(
                "legacy-replace-paper",
                "literature.duplicate-merge.legacy.replace-paper",
                "2005326f1b1403a8dba780de799543ced130b60b0bd0b7431efe31ccd9ff284d",
                &[T, T, T],
                &[T],
            )?,
            semantic_space_policy: load(
                "semantic-space-policy",
                "literature.duplicate-merge.semantic-space-policy",
                "a7dfb370993be275ca9f64b84010001256dce5baa7388aa60a6d3eb103f4893c",
                &[N, N],
                &[T, T, T, I, T, T],
            )?,
            semantic_generation_path: load(
                "semantic-generation-path",
                "literature.duplicate-merge.semantic-generation-path",
                "e92212c0e92808ec254cd09e4b50efee704e039582069d154b7fc7f687fe6264",
                &[T, I],
                &[T],
            )?,
            semantic_record_row_id: load(
                "semantic-record-row-id",
                "literature.duplicate-merge.semantic-record-row-id",
                "c5df48929563a4a7d8e8e0e4382609acfd10a52531ffbdfea8dd9c6a3466baa9",
                &[T, T, I],
                &[T],
            )?,
            semantic_job_idempotency: load(
                "semantic-job-idempotency",
                "literature.duplicate-merge.semantic-job-idempotency",
                "7c6564e8b4c824c67fa88198ed84255320dc72c0dd9072db3c4257a72dc3eae5",
                &[T],
                &[T],
            )?,
            legacy_term_metadata: load(
                "legacy-term-metadata",
                "literature.duplicate-merge.legacy.term-metadata",
                "a0f1566fb2649bbcf10021cbc3c139f634973b5c6ba143c061aeb8d6c3edc1ab",
                &[],
                &[T, T, I, T, T],
            )?,
            legacy_concept_metadata: load(
                "legacy-concept-metadata",
                "literature.duplicate-merge.legacy.concept-metadata",
                "19c857bb987da688a6a94fddf2d19e13f5891743a94a0133802dee051daab732",
                &[],
                &[T, T, I, T, T],
            )?,
            legacy_repair_skip: load(
                "legacy-repair-skip",
                "literature.duplicate-merge.legacy.repair-skip",
                "69b4968029f60b54e019a59fe31f4535a135d4e737843138bd94718e7f68c007",
                &[I],
                &[I],
            )?,
            legacy_concept_delete_error: load(
                "legacy-concept-delete-error",
                "literature.duplicate-merge.legacy.concept-delete-error",
                "e19c45b089ce5610de8c1f607eff39244dccf937c611bbc806de74646f39a93b",
                &[I],
                &[N],
            )?,
            legacy_nal_insert_policy: load(
                "legacy-nal-insert-policy",
                "literature.duplicate-merge.legacy.nal-insert-policy",
                "8b5a590fc4aca17381f50db485997592cd2e90148f31780164b5b4fa038f2701",
                &[T, T, T],
                &[N],
            )?,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::data_interface::plugin_resources::{
        ColumnCodec as C, RowPolicy, read_on_connection,
    };
    use research_workbench_native_sdk::data::{DataCall, DataValue as V};
    use rusqlite::Connection;
    use std::{fs, path::Path};

    pub(crate) fn package() -> (
        tempfile::TempDir,
        RuntimePluginCatalog,
        RuntimePluginPackage,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("literature_library");
        fs::create_dir_all(root.join("data")).unwrap();
        fs::write(
            root.join("manifest.json"),
            r#"{"id":"literature_library","entry":"index.js","styles":[]}"#,
        )
        .unwrap();
        fs::write(root.join("index.js"), "export {};").unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/native-resources-v1/literature");
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.path().extension().is_some_and(|value| value == "sql") {
                fs::copy(entry.path(), root.join("data").join(entry.file_name())).unwrap();
            }
        }
        let catalog = RuntimePluginCatalog::new(Some(directory.path().into()));
        let package = catalog.discover().unwrap().packages.remove(0);
        (directory, catalog, package)
    }

    pub(crate) fn call(resource: &AcceptedDataResource, arguments: Vec<V>) -> DataCall {
        DataCall {
            id: resource.grant().id.clone(),
            version: resource.grant().version,
            arguments,
        }
    }

    #[test]
    fn all_rule_grants_load_and_new_metadata_is_read_from_the_package() {
        let (_directory, catalog, package) = package();
        let resources = MergeRuleResources::load(&catalog, &package).unwrap();
        let db = Connection::open_in_memory().unwrap();
        for (resource, kind, reason) in [
            (&resources.legacy_term_metadata, "term", "manual_create"),
            (
                &resources.legacy_concept_metadata,
                "concept",
                "nal_collection_migration",
            ),
        ] {
            let rows = read_on_connection(
                &db,
                resource,
                &call(resource, vec![]),
                RowPolicy::FirstRequired,
                &[C::String, C::String, C::I64, C::String, C::String],
            )
            .unwrap();
            assert_eq!(
                rows.rows,
                vec![vec![
                    V::Text("human_original".into()),
                    V::Text(kind.into()),
                    V::Integer(1),
                    V::Text("human".into()),
                    V::Text(reason.into())
                ]]
            );
        }
        let resource = &resources.legacy_nal_insert_policy;
        let subject = r#"{"kind":"product","terms":[]}"#;
        let predicate = r#"{"kind":"concept","id":""}"#;
        for (origin, expected) in [("legacy", V::Text(String::new())), (" legacy ", V::Null)] {
            let rows = read_on_connection(
                &db,
                resource,
                &call(
                    resource,
                    vec![
                        V::Text(subject.into()),
                        V::Text(predicate.into()),
                        V::Text(origin.into()),
                    ],
                ),
                RowPolicy::FirstRequired,
                &[C::OptionalString],
            )
            .unwrap();
            assert_eq!(rows.rows, vec![vec![expected]]);
        }
    }
}
