//! Cross-plugin reads share the caller's transaction and the existing L host.
use super::*;
use research_workbench_native_sdk::catalog::{
    CatalogInput, CatalogOutput, CatalogProjectionInput, CatalogProjectionOutcome, CatalogRequest,
    bibliography::{CandidatesInput, CandidatesPlan, CandidatesQuery},
};
use serde_json::Value;
impl Runtime<'_> {
    pub(super) fn library_search(
        &self,
        db: &Connection,
        reader: &mut Reader,
        keys: &str,
        project_only: bool,
        project: &str,
        query: &str,
    ) -> Read<Vec<research_workbench_native_sdk::catalog::bibliography::CitationSearchHit>> {
        use research_workbench_native_sdk::catalog::bibliography::{SearchDecision, SearchInput};
        let holder = self.library.borrow();
        let Some(resources) = holder.as_ref() else {
            return Err(Flow::Need(Need::LibraryResources));
        };
        let prepared = reader.library(
            CatalogInput::BibliographySearch {
                input: Box::new(SearchInput::Prepare {
                    query: query.into(),
                }),
            },
            &resources.key,
        )?;
        let CatalogOutput::BibliographySearch {
            decision: SearchDecision::Needle { needle },
        } = prepared.as_ref()
        else {
            return reader.mismatch();
        };
        let rows = resources
            .search(db, keys, project_only, project, needle)
            .map_err(candidate_data)?;
        let output = reader.library(
            CatalogInput::BibliographySearch {
                input: Box::new(SearchInput::Project { rows }),
            },
            &resources.key,
        )?;
        match output.as_ref() {
            CatalogOutput::BibliographySearch {
                decision: SearchDecision::Hits { hits },
            } => Ok(hits.clone()),
            _ => reader.mismatch(),
        }
    }
    pub(super) fn imported_key(&self, db: &Connection, id: &str) -> Read<Option<String>> {
        let holder = self.library.borrow();
        let Some(resources) = holder.as_ref() else {
            return Err(Flow::Need(Need::LibraryResources));
        };
        resources.imported_key(db, id).map_err(candidate_data)
    }
    pub(super) fn active_library_source(&self, db: &Connection, id: &str) -> Read<bool> {
        if !self.library_available() {
            return Ok(false);
        }
        let holder = self.library.borrow();
        let Some(resources) = holder.as_ref() else {
            return Err(Flow::Need(Need::LibraryResources));
        };
        resources.source_exists(db, id).map_err(candidate_data)
    }
    pub(super) fn resume(&self, reader: &mut Reader, need: Need) -> NativeResult<()> {
        match need {
            Need::Recapture => Ok(()),
            Need::Native(input) => {
                let decided = self.operation.decide(*input)?;
                self.remember(&decided);
                reader.resolve(decided, &self.operation.key)
            }
            Need::LibraryResources => {
                let binding = self
                    .operation
                    .store
                    .native_note_trace()
                    .ok_or_else(|| self.error(WritingError::LiteratureUnavailable))?;
                let resources = self
                    .operation
                    .runtime
                    .block_on(binding.service.bibliography_resources(&binding.package_key))?;
                if resources.key != binding.package_key {
                    return Err(self.mismatch());
                }
                *self.library.borrow_mut() = Some(resources);
                Ok(())
            }
            Need::Library(input) => {
                let binding = self
                    .operation
                    .store
                    .native_note_trace()
                    .ok_or_else(|| self.error(WritingError::LiteratureUnavailable))?;
                let digest = Reader::library_digest(&input).map_err(|flow| match flow {
                    Flow::Fail(cause) => self.failed(cause, reader),
                    Flow::Need(_) => self.mismatch(),
                })?;
                let decided = self.operation.runtime.block_on(
                    binding
                        .service
                        .bibliography_call(&binding.package_key, *input),
                )?;
                reader.resolve_library(digest, decided, &binding.package_key);
                Ok(())
            }
        }
    }
    pub(super) fn library_item(
        &self,
        db: &Connection,
        reader: &mut Reader,
        id: &str,
    ) -> Read<Value> {
        let holder = self.library.borrow();
        let Some(resources) = holder.as_ref() else {
            return Err(Flow::Need(Need::LibraryResources));
        };
        let items = crate::runtime_plugins::literature_catalog::data::capture_items(
            db,
            &resources.catalog,
            &[id.to_owned()],
        )
        .map_err(library_data)?;
        let output = reader.library(
            CatalogInput::Project {
                input: CatalogProjectionInput {
                    request: CatalogRequest::Get { id: id.into() },
                    items,
                },
            },
            &resources.key,
        )?;
        match output.as_ref() {
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::Success { items },
            } if items.len() == 1 && items[0].id == id => {
                serde_json::to_value(&items[0]).map_err(original)
            }
            CatalogOutput::Projection {
                outcome: CatalogProjectionOutcome::NotFound,
            } => reader.rejected(WritingError::BibliographyImport("找不到文献或附件".into())),
            _ => reader.mismatch(),
        }
    }
    pub(super) fn match_library(
        &self,
        db: &Connection,
        reader: &mut Reader,
        draft: &Value,
    ) -> Read<Option<Value>> {
        let doi = crate::doi::normalized_doi(draft["doi"].as_str().unwrap_or_default());
        let title = draft["title"].as_str().unwrap_or_default().to_owned();
        let mut input = CandidatesInput {
            doi: doi.clone(),
            title,
            primary: None,
            fallback: None,
        };
        let (ids, exact_doi) = loop {
            let holder = self.library.borrow();
            let Some(resources) = holder.as_ref() else {
                return Err(Flow::Need(Need::LibraryResources));
            };
            let output = reader.library(
                CatalogInput::PlanBibliographyCandidates {
                    input: Box::new(input.clone()),
                },
                &resources.key,
            )?;
            let CatalogOutput::BibliographyCandidates { plan } = output.as_ref() else {
                return reader.mismatch();
            };
            match plan {
                CandidatesPlan::Query { query, parameter } => {
                    let ids = resources
                        .candidates(db, *query, parameter)
                        .map_err(candidate_data)?;
                    match query {
                        CandidatesQuery::Doi | CandidatesQuery::Title => input.primary = Some(ids),
                        CandidatesQuery::TitleFallback => input.fallback = Some(ids),
                    }
                }
                CandidatesPlan::Ready { ids, exact_doi } => break (ids.clone(), *exact_doi),
            }
        };
        let mut candidates = Vec::with_capacity(ids.len());
        for id in ids {
            candidates.push(self.library_item(db, reader, &id)?);
        }
        match reader.adoption(AdoptionInput::MatchLibrary {
            normalized_doi: doi,
            draft: draft.clone(),
            exact_doi,
            candidates,
        })? {
            AdoptionDecision::LibraryMatched { item } => Ok(item),
            _ => reader.mismatch(),
        }
    }
}
fn library_data(
    cause: Box<crate::data_interface::plugin_resources::PluginDataError>,
) -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::BibliographyImport(format!(
            "文献数据库操作失败：{}",
            cause.safe_cause
        )),
        cause: FailureCause::SourceData(cause),
    })
}
fn candidate_data(
    cause: Box<crate::data_interface::plugin_resources::PluginDataError>,
) -> Flow<WritingError, Need> {
    Flow::Fail(Failure {
        error: WritingError::Storage(crate::storage::StorageError::DataInterface(
            crate::data_interface::DataError::Unavailable,
        )),
        cause: FailureCause::SourceData(cause),
    })
}
