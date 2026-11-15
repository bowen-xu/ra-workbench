//! Exact finite package-owned SQL metadata, frozen from the original reading owner.
use super::*;
use research_workbench_native_sdk::data::{DataAccess, DataField, DataResourceGrant, DataType};
#[derive(Clone, Copy)]
#[repr(usize)]
pub(super) enum Resource {
    AttachmentsPaperExists,
    AttachmentsAll,
    AttachmentsExact,
    SnapshotLoadHeader,
    SnapshotLoadAttachments,
    SnapshotLoadItems,
    SnapshotPersistFindHash,
    SnapshotMembership,
    SnapshotItemGeometry,
    SnapshotRepairDeleteItems,
    SnapshotRepairDeleteAttachments,
    SnapshotRepairDeleteHeader,
    SnapshotPersistInsertHeader,
    SnapshotPersistInsertAttachment,
    SnapshotPersistInsertItem,
}
pub(super) struct Spec {
    pub name: &'static str,
    pub sha: &'static str,
    pub parameters: &'static [DataField],
    pub codecs: &'static [C],
    pub write: bool,
}
const TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: false,
};
const INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: false,
};
const OPTIONAL_TEXT: DataField = DataField {
    data_type: DataType::Text,
    nullable: true,
};
const OPTIONAL_INTEGER: DataField = DataField {
    data_type: DataType::Integer,
    nullable: true,
};
const SPECS: [Spec; 15] = [
    Spec {
        name: "attachments-paper-exists",
        sha: "0d11774cf214b7a223310792c915b535f5fcda72d3f795a9526bd915174b3edf",
        parameters: &[TEXT],
        codecs: &[C::Bool],
        write: false,
    },
    Spec {
        name: "attachments-all",
        sha: "92f40882515e701625d31078c13439982c879162561faa8b1a14df3833e6bd50",
        parameters: &[TEXT],
        codecs: &[C::String, C::String, C::String],
        write: false,
    },
    Spec {
        name: "attachments-exact",
        sha: "db3d31b54f18d626af0a41300080a279dfa627a028a332d77db4833c0010501c",
        parameters: &[TEXT, TEXT],
        codecs: &[C::String, C::String, C::String],
        write: false,
    },
    Spec {
        name: "snapshot-load-header",
        sha: "89c66d6d96446fa73d1d131a4a9c6f200507d394ec7fd1b675f3ccfbb5cca1d3",
        parameters: &[TEXT],
        codecs: &[C::String, C::String, C::String, C::I64, C::String],
        write: false,
    },
    Spec {
        name: "snapshot-load-attachments",
        sha: "b2c9f32eb2e6648463a3759dc5d0825dec0f36d5942a9a684f32620babfded1d",
        parameters: &[TEXT],
        codecs: &[C::String, C::String, C::String, C::I64],
        write: false,
    },
    Spec {
        name: "snapshot-load-items",
        sha: "4763cfa6778a79d1c1043892e136e0b63e327605ae3d65f09dca682cd2f11446",
        parameters: &[TEXT],
        codecs: &[
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::String,
            C::OptionalString,
            C::OptionalI64,
            C::String,
            C::OptionalString,
            C::OptionalString,
            C::OptionalString,
            C::OptionalString,
            C::OptionalString,
        ],
        write: false,
    },
    Spec {
        name: "snapshot-persist-find-hash",
        sha: "c021b0efb0880ed343e54910b95f07bcd7a3cb20847c585ddaa594abf9c5db2d",
        parameters: &[TEXT],
        codecs: &[C::String],
        write: false,
    },
    Spec {
        name: "snapshot-membership",
        sha: "1afd1b147dec7731e81deb9d5b73bb5fd0a24dad2388533ec864bc616d6cbfec",
        parameters: &[TEXT, TEXT],
        codecs: &[C::Bool],
        write: false,
    },
    Spec {
        name: "snapshot-item-geometry",
        sha: "6aac9350194c0edf4c5d1cb60b9a18be050b9845b63c10922cd6052e972808ef",
        parameters: &[TEXT, TEXT],
        codecs: &[C::String, C::String, C::String, C::String],
        write: false,
    },
    Spec {
        name: "snapshot-repair-delete-items",
        sha: "8970fcf52e4e3eadf432e4c42b64665d9ea839c59dfd4c939f67b77c813acdab",
        parameters: &[TEXT],
        codecs: &[],
        write: true,
    },
    Spec {
        name: "snapshot-repair-delete-attachments",
        sha: "da86c8c2d617c11b1aa045bf54358c449c399ee92bac90a9dc7f4d11008e9986",
        parameters: &[TEXT],
        codecs: &[],
        write: true,
    },
    Spec {
        name: "snapshot-repair-delete-header",
        sha: "8ae9b8212b4ee86dc0bda394c341696eabbf76d052bc39d1fe74a61f50d57fa5",
        parameters: &[TEXT],
        codecs: &[],
        write: true,
    },
    Spec {
        name: "snapshot-persist-insert-header",
        sha: "8f9d17e3cb6bafac7ca3b350653f6928bc4467739654c6477757b7b31560ee20",
        parameters: &[TEXT, TEXT, TEXT, INTEGER],
        codecs: &[],
        write: true,
    },
    Spec {
        name: "snapshot-persist-insert-attachment",
        sha: "bf0f9052bdc7fc234c615ac397bc495ab511cc0aa297583dc1fd4fdfa276fc62",
        parameters: &[TEXT, INTEGER, TEXT, TEXT, TEXT],
        codecs: &[],
        write: true,
    },
    Spec {
        name: "snapshot-persist-insert-item",
        sha: "15e1c79d0a044b72734ea1a81f8afc53fbe4704b7c499fe2505c548b629f322c",
        parameters: &[
            TEXT,
            TEXT,
            TEXT,
            TEXT,
            TEXT,
            TEXT,
            TEXT,
            OPTIONAL_TEXT,
            OPTIONAL_INTEGER,
            TEXT,
            OPTIONAL_TEXT,
            OPTIONAL_TEXT,
            OPTIONAL_TEXT,
            OPTIONAL_TEXT,
            OPTIONAL_TEXT,
        ],
        codecs: &[],
        write: true,
    },
];
#[cfg(test)]
pub(super) fn name(index: usize) -> &'static str {
    SPECS[index].name
}
impl Resource {
    pub(super) fn spec(self) -> &'static Spec {
        &SPECS[self as usize]
    }
}
pub(super) fn load(
    catalog: &RuntimePluginCatalog,
    package: &RuntimePluginPackage,
) -> Result<Vec<AcceptedDataResource>, RuntimePluginError> {
    SPECS
        .iter()
        .map(|spec| {
            AcceptedDataResource::load(
                catalog,
                package,
                "literature_library",
                &format!("data/paper-context-{}.v1.sql", spec.name),
                DataResourceGrant {
                    id: format!("literature.paper-context.{}", spec.name),
                    version: 1,
                    sha256: spec.sha.into(),
                    access: if spec.write {
                        DataAccess::Write
                    } else {
                        DataAccess::Read
                    },
                    parameters: spec.parameters.to_vec(),
                    columns: spec.codecs.iter().map(|codec| codec.field()).collect(),
                },
            )
        })
        .collect()
}
