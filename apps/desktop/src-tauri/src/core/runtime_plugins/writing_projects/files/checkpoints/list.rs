//! Original ordered reads, followed by one complete source recheck.
use super::*;
use research_workbench_native_sdk::{
    data::DataValue,
    streaming_payload::{self, PayloadDigest},
};
use rusqlite::Connection;
struct CapturedRecord {
    digest: PayloadDigest,
    record: CheckpointRecord,
}
impl FileOperation {
    pub(in crate::runtime_plugins::writing_projects::files) fn checkpoint_list(
        &self,
        resources: &Resources,
    ) -> NativeResult<Vec<CheckpointRecord>> {
        loop {
            let manuscript = {
                let db = self
                    .store
                    .connection()
                    .map_err(|cause| failure("storage_error", cause.to_string(), &self.key))?;
                self.resources
                    .projects
                    .get(&db, &self.manuscript_id)
                    .map_err(data_failure)?
            };
            let project = self.checkpoint_project(manuscript.clone())?;
            let paths = (|| {
                let db = self
                    .store
                    .connection()
                    .map_err(|cause| failure("storage_error", cause.to_string(), &self.key))?;
                if self
                    .resources
                    .projects
                    .get(&db, &self.manuscript_id)
                    .map_err(data_failure)?
                    != manuscript
                {
                    return Ok(None);
                }
                self.checkpoint_paths(resources, &db).map(Some)
            })()
            .map_err(|error| project.origin.attach(error))?;
            let Some(paths) = paths else {
                continue;
            };
            let mut captured = Vec::with_capacity(paths.len());
            let mut last_origin = project.origin;
            for path in &paths {
                let rows = (|| {
                    let db = self
                        .store
                        .connection()
                        .map_err(|cause| failure("storage_error", cause.to_string(), &self.key))?;
                    resources
                        .read(&db, &self.manuscript_id, path)
                        .map_err(data_failure)
                })()
                .map_err(|error| last_origin.attach(error))?;
                let digest = self
                    .checkpoint_digest(&rows)
                    .map_err(|error| last_origin.attach(error))?;
                let Ready { value, origin } = self.checkpoint_read(path, true, rows)?;
                let record = value.ok_or_else(|| origin.attach(self.checkpoint_mismatch()))?;
                last_origin = origin;
                captured.push(CapturedRecord { digest, record });
            }
            if self
                .checkpoint_list_current(resources, &manuscript, &paths, &captured)
                .map_err(|error| last_origin.attach(error))?
            {
                return Ok(captured
                    .into_iter()
                    .map(|captured| captured.record)
                    .collect());
            }
        }
    }
    fn checkpoint_paths(
        &self,
        resources: &Resources,
        db: &Connection,
    ) -> NativeResult<Vec<String>> {
        resources
            .paths(db, &self.manuscript_id)
            .map_err(data_failure)?
            .rows
            .into_iter()
            .map(|row| match row.as_slice() {
                [DataValue::Text(path)] => Ok(path.clone()),
                _ => Err(self.checkpoint_mismatch()),
            })
            .collect()
    }
    fn checkpoint_digest(&self, rows: &DataRows) -> NativeResult<PayloadDigest> {
        streaming_payload::write_json(std::io::sink(), rows)
            .map(|(_, digest)| digest)
            .map_err(|cause| {
                failure(
                    "storage_error",
                    format!("writing_checkpoint_read_fingerprint_failed：{cause}"),
                    &self.key,
                )
            })
    }
    fn checkpoint_list_current(
        &self,
        resources: &Resources,
        manuscript: &DataRows,
        paths: &[String],
        captured: &[CapturedRecord],
    ) -> NativeResult<bool> {
        let db = self
            .store
            .connection()
            .map_err(|cause| failure("storage_error", cause.to_string(), &self.key))?;
        if self
            .resources
            .projects
            .get(&db, &self.manuscript_id)
            .map_err(data_failure)?
            != *manuscript
            || self.checkpoint_paths(resources, &db)? != paths
        {
            return Ok(false);
        }
        for entry in captured {
            let rows = resources
                .read(
                    &db,
                    &self.manuscript_id,
                    &entry.record.checkpoint.relative_path,
                )
                .map_err(data_failure)?;
            let digest = self.checkpoint_digest(&rows)?;
            if entry.digest.byte_size != digest.byte_size || entry.digest.sha256 != digest.sha256 {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
