update derived_research_views set view_kind = ?2, title = ?3, scope_note = ?4,
             inputs_json = ?5, items_json = ?6, review_state = ?7,
             provenance_kind = ?8, current_revision = ?9,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and current_revision = ?10
