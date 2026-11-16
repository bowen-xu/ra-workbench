update structured_reading_units
                 set project_id = ?2, paper_id = ?3, unit_kind = ?4, title = ?5, body = ?6,
                     provenance_kind = ?7, epistemic_role = ?8, revision = ?9,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 where id = ?1 and revision = ?10 and reverted_at is null
