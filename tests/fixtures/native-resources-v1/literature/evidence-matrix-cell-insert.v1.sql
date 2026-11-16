insert into evidence_matrix_cells
         (id, matrix_id, paper_id, dimension_id, coverage_state, coverage_note,
          statements_json, provenance_kind, current_revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'human_original', 1,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
