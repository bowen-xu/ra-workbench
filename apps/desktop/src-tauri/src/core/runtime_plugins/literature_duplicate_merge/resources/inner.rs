//! Fixed original inner merge IO resources; no domain policy or transaction owner.
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

#[derive(Clone)]
pub(crate) struct InnerMergeResources {
    pub(crate) collection_revision: AcceptedDataResource,
    pub(crate) collection_collections: AcceptedDataResource,
    pub(crate) collection_types: AcceptedDataResource,
    pub(crate) collection_relations: AcceptedDataResource,
    pub(crate) collection_memberships: AcceptedDataResource,
    pub(crate) collection_assignments: AcceptedDataResource,
    pub(crate) collection_scope: AcceptedDataResource,
    pub(crate) statement_scope: AcceptedDataResource,
    pub(crate) annotation_scope: AcceptedDataResource,
    pub(crate) nal_revision_increment: AcceptedDataResource,
    pub(crate) collection_revision_increment: AcceptedDataResource,
    pub(crate) statement_relation_kind: AcceptedDataResource,
    pub(crate) statement_insert: AcceptedDataResource,
    pub(crate) copy_memberships: AcceptedDataResource,
    pub(crate) copy_assignments: AcceptedDataResource,
    pub(crate) nal_target: AcceptedDataResource,
    pub(crate) insert_annotation: AcceptedDataResource,
    pub(crate) nal_annotation_notes: AcceptedDataResource,
}

impl InnerMergeResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let load = |file: &str,
                    id: &str,
                    hash: &str,
                    access: DataAccess,
                    parameters: &[DataField],
                    columns: &[DataField]| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/{file}.v1.sql"),
                DataResourceGrant {
                    id: id.into(),
                    version: 1,
                    sha256: hash.into(),
                    access,
                    parameters: parameters.to_vec(),
                    columns: columns.to_vec(),
                },
            )
        };
        Ok(Self {
            collection_revision: load(
                "duplicate-undo-collection-revision",
                "literature.duplicate-undo.collection-revision",
                "0638f464e3ed8b6b01a469fe41e980eb8b1c6e5445b6b3c2becc9d15649cc3d0",
                DataAccess::Read,
                &[],
                &[I],
            )?,
            collection_collections: load(
                "duplicate-undo-collection-collections",
                "literature.duplicate-undo.collection-collections",
                "c31f3be5c7b06c8211f360cd46a5ea303ae9971f0291f36007a7fe9612aed787",
                DataAccess::Read,
                &[],
                &[T, T, T, N],
            )?,
            collection_types: load(
                "duplicate-undo-collection-types",
                "literature.duplicate-undo.collection-types",
                "ebe6c7c433c7ae75de58853efada58d13eb1e775c3624627f3ac4d8a63856f17",
                DataAccess::Read,
                &[],
                &[T, T, T, I, I],
            )?,
            collection_relations: load(
                "duplicate-undo-collection-relations",
                "literature.duplicate-undo.collection-relations",
                "de7d852f700d9ddb6775fe34f0e2dea6ec11f29b517fb0e580529f2b6ba6c2ce",
                DataAccess::Read,
                &[],
                &[T, T, T, T],
            )?,
            collection_memberships: load(
                "duplicate-undo-collection-memberships",
                "literature.duplicate-undo.collection-memberships",
                "5476cf7f3a248d6045825e6c81274f684649a5aacc9d2bc01b875dd8d2743fb4",
                DataAccess::Read,
                &[],
                &[T, T],
            )?,
            collection_assignments: load(
                "duplicate-undo-collection-assignments",
                "literature.duplicate-undo.collection-assignments",
                "d4863f31b212bb8fd51f0bbf33ad9d624fcbac6a9cacc01a624ad25aa5b77f2e",
                DataAccess::Read,
                &[],
                &[T, T],
            )?,
            collection_scope: load(
                "duplicate-undo-collection-scope",
                "literature.duplicate-undo.collection-scope",
                "55ab9580b79f8c70b8ed40c1a4717541ec530eb5013b1bd49dd65b709173798a",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            statement_scope: load(
                "duplicate-undo-statement-scope",
                "literature.duplicate-undo.statement-scope",
                "2ae437cc4a76b2f907a97092c0f5bc11343f63a30781d57d91a9c83cbb53489d",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            annotation_scope: load(
                "duplicate-undo-annotation-scope",
                "literature.duplicate-undo.annotation-scope",
                "f3d82c7a2a5dd143c2d6b0f1ec08e87fa93ed3fc22617220aae9aa87c8a01908",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            nal_revision_increment: load(
                "duplicate-undo-increment-nal-revision",
                "literature.duplicate-undo.increment-nal-revision",
                "5a3bbfaec5a693ea4f8161f938ef9e06062ed8e76783e3ca5f536283ba731100",
                DataAccess::Write,
                &[],
                &[],
            )?,
            collection_revision_increment: load(
                "duplicate-undo-increment-collection-revision",
                "literature.duplicate-undo.increment-collection-revision",
                "12ea366dcd0e087573739546fb896d833fbe8c049d7f79f6f3af26c9c989157e",
                DataAccess::Write,
                &[],
                &[],
            )?,
            statement_relation_kind: load(
                "nal-bootstrap-statement-relation-kind",
                "literature.nal-bootstrap.statement-relation-kind",
                "81252276aae029b227371656d8b684cd61aeffa3d2bab72542426076d19712c6",
                DataAccess::Write,
                &[T],
                &[],
            )?,
            statement_insert: load(
                "nal-bootstrap-statement-insert",
                "literature.nal-bootstrap.statement-insert",
                "e9c07790e1f98f64b29b55c3aecf5422a4e11d2fe2a417c53b4627223a29fdbc",
                DataAccess::Write,
                &[T, T, T, T],
                &[],
            )?,
            copy_memberships: load(
                "duplicate-merge-copy-memberships",
                "literature.duplicate-merge.copy-memberships",
                "7d6b25628694788c4004d77691746f2e8af27d0333780797d593fdec6612f06e",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            copy_assignments: load(
                "duplicate-merge-copy-assignments",
                "literature.duplicate-merge.copy-assignments",
                "01062770abcae65dcdff59a5ab92cdbed2dc0e7ccead3435179047bd4fdd84b5",
                DataAccess::Write,
                &[T, T],
                &[],
            )?,
            nal_target: load(
                "duplicate-merge-nal-target",
                "literature.duplicate-merge.nal-target",
                "35989481d992941214496f4fc52cbf96a3f989bd6f45834b4474bf2a7bc8e090",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
            insert_annotation: load(
                "duplicate-merge-insert-annotation",
                "literature.duplicate-merge.insert-annotation",
                "26e6cccc743bcad1411a865c799b57cd1e5a37cf306a53ddc3eca54db7d593b2",
                DataAccess::Write,
                &[T, T, T],
                &[],
            )?,
            nal_annotation_notes: load(
                "duplicate-merge-nal-annotation-notes",
                "literature.duplicate-merge.nal-annotation-notes",
                "ce2c0c45ab08fa9494e6136e742e219becf7a9b76bca46b3ef736f439033e5e2",
                DataAccess::Read,
                &[T, T],
                &[T],
            )?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::rules::tests::{call, package};
    use super::*;
    use crate::data_interface::plugin_resources::{
        ColumnCodec as C, RowPolicy, read_on_connection, write_in,
    };
    use research_workbench_native_sdk::data::DataValue as V;
    use rusqlite::Connection;

    #[test]
    fn actual_copy_resources_keep_conflict_target_and_source_annotation_order() {
        let (_directory, catalog, package) = package();
        let resources = InnerMergeResources::load(&catalog, &package).unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("create table collection_memberships(item_id text,collection_id text,unique(item_id,collection_id));
            create table item_relation_assignments(item_id text,relation_id text,unique(item_id,relation_id));
            create table nal_statements(id text,subject text,predicate text,origin text,unique(subject,predicate));
            insert into collection_memberships values('source','a'),('source','b'),('survivor','a');
            insert into item_relation_assignments values('source','r');").unwrap();
        let subject = r#"{"kind":"paper","id":"survivor"}"#;
        let predicate = r#"{"kind":"concept","id":"target"}"#;
        db.execute(
            "insert into nal_statements values('original-target',?1,?2,'human')",
            [subject, predicate],
        )
        .unwrap();
        let tx = db.transaction().unwrap();
        for resource in [&resources.copy_memberships, &resources.copy_assignments] {
            assert_eq!(
                write_in(
                    &tx,
                    resource,
                    &call(
                        resource,
                        vec![V::Text("survivor".into()), V::Text("source".into())]
                    )
                )
                .unwrap(),
                1
            );
        }
        let resource = &resources.statement_insert;
        assert_eq!(
            write_in(
                &tx,
                resource,
                &call(
                    resource,
                    vec![
                        V::Text("attempted-new-id".into()),
                        V::Text(subject.into()),
                        V::Text(predicate.into()),
                        V::Text("legacy".into())
                    ]
                )
            )
            .unwrap(),
            0
        );
        let resource = &resources.nal_target;
        let rows = read_on_connection(
            &tx,
            resource,
            &call(
                resource,
                vec![V::Text(subject.into()), V::Text(predicate.into())],
            ),
            RowPolicy::FirstRequired,
            &[C::String],
        )
        .unwrap();
        assert_eq!(rows.rows, vec![vec![V::Text("original-target".into())]]);
        let annotations = serde_json::json!([
            {"id":"a","targetKind":"statement","targetId":"other","notePageId":"ignored"},
            {"id":"b","targetKind":"statement","targetId":"source-statement","notePageId":"second"},
            {"id":"c","targetKind":"concept","targetId":"source-statement","notePageId":"second"},
            {"id":"d","targetKind":"statement","targetId":"source-statement","notePageId":""}
        ]);
        // The caller already scoped this vector. This fixed selector must not
        // silently add a second target-kind filter, DISTINCT or a note check.
        let resource = &resources.nal_annotation_notes;
        let rows = read_on_connection(
            &tx,
            resource,
            &call(
                resource,
                vec![
                    V::Text(annotations.to_string()),
                    V::Text("source-statement".into()),
                ],
            ),
            RowPolicy::All,
            &[C::String],
        )
        .unwrap();
        assert_eq!(
            rows.rows,
            vec![
                vec![V::Text("second".into())],
                vec![V::Text("second".into())],
                vec![V::Text(String::new())]
            ]
        );
        tx.commit().unwrap();
        assert_eq!(
            db.query_row(
                "select count(*) from collection_memberships where item_id='survivor'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            2
        );
        assert_eq!(
            db.query_row(
                "select origin from nal_statements where id='original-target'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "human"
        );
    }
}
