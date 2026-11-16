update evidence_matrix_cells set coverage_state = ?2, coverage_note = ?3,
             statements_json = ?4, current_revision = ?5,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and current_revision = ?6
