use super::*;
use crate::data_interface::plugin_resources::{MappedReadError, read_on_connection_mapped};
impl PaperContextResources {
    pub(crate) fn capture_snapshot(
        &self,
        db: &Connection,
        projection: &SnapshotProjectionV1,
    ) -> ContextResult<SnapshotCapture> {
        let hash_rows = self
            .read(
                db,
                R::SnapshotPersistFindHash,
                vec![text(&projection.context_hash)],
                P::FirstOptional,
            )?
            .rows;
        let Some(row) = hash_rows.first() else {
            return Ok(SnapshotCapture {
                existing: None,
                witness: SnapshotWitness {
                    hash_rows,
                    header: vec![],
                    attachments: vec![],
                    items: vec![],
                },
            });
        };
        let [V::Text(id)] = row.as_slice() else {
            return Err(self.shape(R::SnapshotPersistFindHash));
        };
        let (snapshot, mut witness) = self.read_snapshot(db, id)?;
        witness.hash_rows = hash_rows;
        Ok(SnapshotCapture {
            existing: Some(snapshot),
            witness,
        })
    }
    pub(crate) fn load_snapshot(
        &self,
        db: &Connection,
        id: &str,
    ) -> ContextResult<PaperContextSnapshot> {
        self.read_snapshot(db, id).map(|(snapshot, _)| snapshot)
    }
    pub(crate) fn read_snapshot(
        &self,
        db: &Connection,
        id: &str,
    ) -> ContextResult<(PaperContextSnapshot, SnapshotWitness)> {
        let header = self
            .read(db, R::SnapshotLoadHeader, vec![text(id)], P::FirstOptional)?
            .rows;
        let Some(row) = header.first() else {
            return Err(failure::invalid("找不到论文上下文快照"));
        };
        let [
            V::Text(id),
            V::Text(paper),
            V::Text(hash),
            V::Integer(schema),
            V::Text(created),
        ] = row.as_slice()
        else {
            return Err(self.shape(R::SnapshotLoadHeader));
        };
        let attachments = self
            .read(db, R::SnapshotLoadAttachments, vec![text(id)], P::All)?
            .rows;
        let decoded_attachments = attachments
            .iter()
            .map(|row| match row.as_slice() {
                [
                    V::Text(attachment),
                    V::Text(sha),
                    V::Text(parse),
                    V::Integer(ordinal),
                ] => Ok(PaperContextAttachment {
                    attachment_id: attachment.clone(),
                    content_sha256: sha.clone(),
                    parse_run_id: parse.clone(),
                    ordinal: *ordinal,
                }),
                _ => Err(self.shape(R::SnapshotLoadAttachments)),
            })
            .collect::<ContextResult<Vec<_>>>()?;
        let resource = self.resource(R::SnapshotLoadItems);
        let mut items = Vec::new();
        // Decode this row's JSON before advancing to the next SQL row, exactly
        // as the original snapshot reader. Retain raw bytes in the witness.
        let decoded = read_on_connection_mapped(
            db,
            resource,
            &self.call(R::SnapshotLoadItems, vec![text(id)]),
            P::All,
            R::SnapshotLoadItems.spec().codecs,
            |row| {
                let item = self.decode_item(&row)?;
                items.push(row);
                Ok(item)
            },
        )
        .map_err(|error| match error {
            MappedReadError::Data(error) => failure::data(error),
            MappedReadError::Mapping(error) => error,
        })?;
        let snapshot = PaperContextSnapshot {
            id: id.clone(),
            paper_id: paper.clone(),
            context_hash: hash.clone(),
            context_schema_version: *schema,
            created_at: created.clone(),
            attachments: decoded_attachments,
            items: decoded,
        };
        Ok((
            snapshot,
            SnapshotWitness {
                hash_rows: vec![],
                header,
                attachments,
                items,
            },
        ))
    }
    fn decode_item(&self, row: &[V]) -> ContextResult<PaperContextItem> {
        let [
            V::Text(id),
            V::Text(attachment),
            V::Text(sha),
            V::Text(parse),
            V::Text(raw),
            V::Text(kind),
            label,
            page,
            V::Text(regions),
            charspan,
            source_hash,
            body,
            table,
            image,
        ] = row
        else {
            return Err(self.shape(R::SnapshotLoadItems));
        };
        let unavailable = || failure::original(AiError::TemporaryUnavailable);
        Ok(PaperContextItem {
            id: id.clone(),
            attachment_id: attachment.clone(),
            content_sha256: sha.clone(),
            parse_run_id: parse.clone(),
            raw_ref: raw.clone(),
            kind: kind.clone(),
            label: optional_text(label),
            page_index: match page {
                V::Integer(value) => u32::try_from(*value).ok(),
                _ => None,
            },
            regions: serde_json::from_str(regions).map_err(|_| unavailable())?,
            charspan: optional_text(charspan)
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|_| unavailable())?,
            source_text_hash: optional_text(source_hash),
            text: optional_text(body),
            table_json: optional_text(table)
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|_| unavailable())?,
            image_content_sha256: optional_text(image),
        })
    }
}
fn optional_text(value: &V) -> Option<String> {
    match value {
        V::Text(value) => Some(value.clone()),
        _ => None,
    }
}
