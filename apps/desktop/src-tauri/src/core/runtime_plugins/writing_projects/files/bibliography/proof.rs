use super::*;
use research_workbench_native_sdk::data::DataRows;
const QUERIES: [&str; 4] = [
    "project-bibliography-proof-files.v1.sql",
    "project-bibliography-proof-entries.v1.sql",
    "project-bibliography-proof-members.v1.sql",
    "project-bibliography-proof-bindings.v1.sql",
];
struct Facts {
    manuscript: DataRows,
    files: DataRows,
    proof: PublicationProof,
}
impl Runtime<'_> {
    fn proof_rows(&self, db: &Connection) -> Result<[Vec<Vec<Option<String>>>; 4], WritingError> {
        let mut captured = Vec::with_capacity(4);
        for name in QUERIES {
            let mut statement = self.sql.prepare(db, name)?;
            let columns = statement.column_count();
            let mut query = statement.query([&self.operation.manuscript_id])?;
            let mut rows = Vec::new();
            while let Some(row) = query.next()? {
                rows.push(
                    (0..columns)
                        .map(|column| row.get::<_, Option<String>>(column))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            captured.push(rows);
        }
        Ok(captured.try_into().expect("four reviewed proof queries"))
    }
    #[cfg(not(unix))]
    pub(super) fn fingerprint(&self) -> NativeResult<Option<[u8; 32]>> {
        Ok(None)
    }

    #[cfg(unix)]
    pub(super) fn fingerprint(&self) -> NativeResult<Option<[u8; 32]>> {
        let facts = self.read(|db, reader| {
            let (project, manuscript) = self.project(db, reader)?;
            let rows = self.proof_rows(db).map_err(original)?;
            let files = self
                .operation
                .resources
                .files(db, &self.operation.manuscript_id)
                .map_err(|cause| {
                    Flow::Fail(Failure {
                        error: WritingError::Storage(crate::storage::StorageError::DataInterface(
                            crate::data_interface::DataError::Unavailable,
                        )),
                        cause: FailureCause::SourceData(cause),
                    })
                })?;
            let decision = reader.call(Input::ListFiles {
                manuscript_id: self.operation.manuscript_id.clone(),
                manuscript: manuscript.clone(),
                files: Some(files.clone()),
            })?;
            let Decision::Files { files: decoded } = decision.as_ref() else {
                return reader.mismatch();
            };
            Ok(Facts {
                manuscript,
                files,
                proof: PublicationProof {
                    root: project.root_path,
                    files: decoded.clone(),
                    rows,
                    stats: SourceStats::Unqueried,
                },
            })
        })?;
        let PublicationDecision::NeedStats { paths } =
            self.publication_decision(PublicationInput::Proof {
                input: Box::new(facts.proof.clone()),
            })?
        else {
            return Err(self.mismatch());
        };
        let Some(stats) = self.source_stats(&paths) else {
            return Ok(None);
        };
        let mut proof = facts.proof.clone();
        proof.stats = SourceStats::Ready {
            values: stats.clone(),
        };
        let PublicationDecision::Proof { fingerprint } =
            self.publication_decision(PublicationInput::Proof {
                input: Box::new(proof),
            })?
        else {
            return Err(self.mismatch());
        };
        {
            let db = self.operation.store.connection().map_err(|cause| {
                failure("storage_error", cause.to_string(), &self.operation.key)
            })?;
            if self
                .operation
                .resources
                .projects
                .get(&db, &self.operation.manuscript_id)
                .map_err(data_failure)?
                != facts.manuscript
                || self.proof_rows(&db).map_err(|cause| self.error(cause))? != facts.proof.rows
                || self
                    .operation
                    .resources
                    .files(&db, &self.operation.manuscript_id)
                    .map_err(data_failure)?
                    != facts.files
            {
                return Ok(None);
            }
        }
        if self.source_stats(&paths).as_ref() != Some(&stats) {
            return Ok(None);
        }
        Ok(fingerprint)
    }
    #[cfg(unix)]
    fn source_stats(&self, paths: &[String]) -> Option<Vec<SourceStat>> {
        let mut values = Vec::with_capacity(paths.len());
        for path in paths {
            let stat = self
                .operation
                .store
                .manuscript_files(&self.operation.manuscript_id)
                .stat(path)
                .ok()?;
            values.push(SourceStat {
                path: path.clone(),
                dev: stat.dev,
                ino: stat.ino,
                len: stat.len,
                mtime: stat.mtime,
                mtime_nsec: stat.mtime_nsec,
                ctime: stat.ctime,
                ctime_nsec: stat.ctime_nsec,
            });
        }
        Some(values)
    }
}
