//! Metadata-only admission prevents new writes from racing the legacy import barrier.
use super::{Error, legacy};
use rusqlite::Connection;

pub(super) fn has_legacy_data(db: &Connection, paper: &str) -> Result<bool, Error> {
    Ok(db.query_row("select exists(select 1 from paper_reading_profile_selections where paper_id=?1 and json_array_length(profile_refs_json)>0) or exists(select 1 from structured_reading_units where paper_id=?1 and reverted_at is null) or exists(select 1 from ai_candidates c where c.candidate_kind in ('paper_portrait_v1','paper_overview_unit_v1') and exists(select 1 from ai_evidence_items e where e.bundle_id=c.evidence_bundle_id and e.paper_id=?1) and not exists(select 1 from ai_evidence_items e where e.bundle_id=c.evidence_bundle_id and e.paper_id!=?1))",[paper],|row|row.get(0))?)
}
pub(crate) fn require_legacy_imported(db: &Connection, paper: &str) -> Result<(), Error> {
    if !legacy::imported(db, &format!("paper:{paper}"))? && has_legacy_data(db, paper)? {
        return Err(Error::LegacyImportRequired);
    }
    Ok(())
}
pub(super) fn require_untouched(db: &Connection, paper: &str) -> Result<(), Error> {
    if has_legacy_data(db,paper)?&&db.query_row("select exists(select 1 from structured_resources r left join structured_paper_template_owners o on o.template_id=r.id where coalesce(r.paper_id,o.paper_id)=?1 and r.revision>0)",[paper],|row|row.get::<_,bool>(0))?{return Err(Error::LegacyImportConflict);}
    Ok(())
}
pub(super) fn mark_no_legacy(db: &Connection, paper: &str) -> Result<(), Error> {
    require_legacy_imported(db, paper)?;
    legacy::mark(db, &format!("paper:{paper}"))
}
