//! Original ordered preview reads and Core-local witnesses, without domain decisions.
use super::*;
use crate::ai::structured_resource_validation::{Failure, FailureCause};
use research_workbench_native_sdk::{
    data::{DataAccess, DataField, DataResourceGrant, DataType},
    paper_context::SnapshotDigestInputV1,
    paper_context_preview::{
        PaperContextError, PaperContextPreviewInputV1 as Input, PaperContextRequest,
    },
};
pub(crate) type PreviewReadFailure = Failure<PaperContextError>;
#[derive(Clone)]
pub(crate) struct PreviewResources {
    context: PaperContextResources,
    resources: Vec<AcceptedDataResource>,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreviewCapture {
    pub input: Input,
    witness: Option<SnapshotWitness>,
}
const SPECS: [(&str, &str, &[C]); 3] = [
    (
        "preview-header",
        "8593ed6b370bef89103628a517a1ea58dfd0f0632e4fcb6d5c50161a7a94c68c",
        &[C::String, C::String, C::I64],
    ),
    (
        "preview-items-budget",
        "1db71f102eb62bb47f1703116b1fef25de31184eac8475dd8eec0dd16ceb4d72",
        &[C::Usize, C::Usize, C::Usize],
    ),
    (
        "preview-attachments-budget",
        "6234a0c4d6bbbf4b226e23bf80eb063dc209f8487de3d2f01da3c88f79be753d",
        &[C::Usize, C::Usize],
    ),
];
impl PreviewResources {
    pub(crate) fn load(
        catalog: &RuntimePluginCatalog,
        package: &RuntimePluginPackage,
    ) -> Result<Self, RuntimePluginError> {
        let resources = SPECS
            .iter()
            .map(|(name, sha, codecs)| {
                AcceptedDataResource::load(
                    catalog,
                    package,
                    "literature_library",
                    &format!("data/paper-context-{name}.v1.sql"),
                    DataResourceGrant {
                        id: format!("literature.paper-context.{name}"),
                        version: 1,
                        sha256: (*sha).into(),
                        access: DataAccess::Read,
                        parameters: vec![DataField {
                            data_type: DataType::Text,
                            nullable: false,
                        }],
                        columns: codecs.iter().map(|codec| codec.field()).collect(),
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            context: PaperContextResources::load(catalog, package)?,
            resources,
        })
    }
    fn read(
        &self,
        db: &Connection,
        index: usize,
        snapshot: &str,
        policy: P,
        error: PaperContextError,
    ) -> Result<Vec<Vec<V>>, PreviewReadFailure> {
        let resource = &self.resources[index];
        read_on_connection(
            db,
            resource,
            &DataCall {
                id: resource.grant().id.clone(),
                version: 1,
                arguments: vec![text(snapshot)],
            },
            policy,
            SPECS[index].2,
        )
        .map(|rows| rows.rows)
        .map_err(|cause| Failure {
            error,
            cause: FailureCause::SourceData(cause),
        })
    }
    pub(crate) fn header(
        &self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> Result<PreviewCapture, PreviewReadFailure> {
        let rows = self.read(
            db,
            0,
            &request.snapshot_id,
            P::FirstOptional,
            PaperContextError::Unavailable,
        )?;
        let header = match rows.as_slice() {
            [] => None,
            [row] => match row.as_slice() {
                [V::Text(paper), V::Text(hash), V::Integer(schema)] => {
                    Some((paper.clone(), hash.clone(), *schema))
                }
                _ => return Err(Failure::original(PaperContextError::Unavailable)),
            },
            _ => return Err(Failure::original(PaperContextError::Unavailable)),
        };
        Ok(PreviewCapture {
            input: Input::Header {
                request: request.clone(),
                header,
            },
            witness: None,
        })
    }
    pub(crate) fn preflight(
        &self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> Result<PreviewCapture, PreviewReadFailure> {
        let items = self.read(
            db,
            1,
            &request.snapshot_id,
            P::FirstRequired,
            PaperContextError::Corrupt,
        )?;
        let attachments = self.read(
            db,
            2,
            &request.snapshot_id,
            P::FirstRequired,
            PaperContextError::Corrupt,
        )?;
        let [item] = items.as_slice() else {
            return Err(Failure::original(PaperContextError::Corrupt));
        };
        let [V::Integer(count), V::Integer(bytes), V::Integer(invalid)] = item.as_slice() else {
            return Err(Failure::original(PaperContextError::Corrupt));
        };
        let [attachment] = attachments.as_slice() else {
            return Err(Failure::original(PaperContextError::Corrupt));
        };
        let [V::Integer(attachment_count), V::Integer(attachment_bytes)] = attachment.as_slice()
        else {
            return Err(Failure::original(PaperContextError::Corrupt));
        };
        Ok(PreviewCapture {
            input: Input::Preflight {
                item_count: *count as usize,
                item_bytes: *bytes as usize,
                invalid_page_count: *invalid as usize,
                attachment_count: *attachment_count as usize,
                attachment_bytes: *attachment_bytes as usize,
            },
            witness: None,
        })
    }
    pub(crate) fn snapshot(
        &self,
        db: &Connection,
        request: &PaperContextRequest,
    ) -> Result<PreviewCapture, PreviewReadFailure> {
        let (snapshot, witness) = self
            .context
            .read_snapshot(db, &request.snapshot_id)
            .map_err(|failure| failure.failure.map_error(|_| PaperContextError::Corrupt))?;
        Ok(PreviewCapture {
            input: Input::Snapshot {
                request: request.clone(),
                snapshot: SnapshotDigestInputV1 {
                    context_schema_version: snapshot.context_schema_version,
                    paper_id: snapshot.paper_id,
                    attachments: snapshot.attachments,
                    items: snapshot.items,
                },
            },
            witness: Some(witness),
        })
    }
}
