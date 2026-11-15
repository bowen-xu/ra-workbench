use super::*;
use load::{encode, text};

impl Execution<'_, '_> {
    pub(super) fn restore(&self) -> Result<()> {
        let r = self.resources;
        let c = self.captured;
        let s = &c.snapshot;
        let phase = Phase::Restore;
        for id in &s.anchor_ids {
            self.write(
                phase,
                &r.restore_anchor,
                vec![text(id), text(&c.merged_item_id), text(&c.survivor_item_id)],
            )?;
        }
        self.write(
            phase,
            &r.restore_reading_sources,
            vec![
                text(&c.merged_item_id),
                text(&encode(&s.anchor_ids, phase)?),
            ],
        )?;
        for id in &s.attachment_ids {
            self.write(
                phase,
                &r.restore_attachment,
                vec![text(id), text(&c.merged_item_id), text(&c.survivor_item_id)],
            )?;
            self.write(
                phase,
                &r.restore_reading_state,
                vec![text(id), text(&c.merged_item_id), text(&c.survivor_item_id)],
            )?;
        }
        for id in &s.identifier_ids {
            self.write(
                phase,
                &r.restore_identifier,
                vec![text(id), text(&c.merged_item_id), text(&c.survivor_item_id)],
            )?;
        }
        for id in &s.added_project_ids {
            self.write(
                phase,
                &r.delete_project,
                vec![text(id), text(&c.survivor_item_id)],
            )?;
        }
        for id in &s.added_tag_ids {
            self.write(
                phase,
                &r.delete_tag,
                vec![text(id), text(&c.survivor_item_id)],
            )?;
        }
        for id in &s.added_provenance_fields {
            self.write(
                phase,
                &r.delete_provenance,
                vec![text(id), text(&c.survivor_item_id)],
            )?;
        }
        let archived = s
            .merged_archived_at
            .as_deref()
            .map(text)
            .unwrap_or(DataValue::Null);
        self.write(
            phase,
            &r.restore_archive,
            vec![text(&c.merged_item_id), archived],
        )?;
        self.write(phase, &r.reset_candidate, vec![text(&s.candidate_id)])?;
        self.write(phase, &r.mark_undone, vec![text(&c.operation_id)])?;
        for id in [&c.survivor_item_id, &c.merged_item_id] {
            self.write(Phase::Search, &r.search_delete, vec![text(id)])?;
            self.write(Phase::Search, &r.search_insert, vec![text(id)])?;
        }
        Ok(())
    }
}
