update structured_reading_units set project_id = ?2, paper_id = ?3,
                 unit_kind = ?4, title = ?5, body = ?6, epistemic_role = ?7,
                 revision = ?8, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             where id = ?1