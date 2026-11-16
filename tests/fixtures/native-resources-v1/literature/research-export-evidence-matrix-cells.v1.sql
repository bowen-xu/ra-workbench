select id, matrix_id, paper_id, dimension_id, coverage_state, coverage_note,
                     statements_json, provenance_kind, current_revision, created_at, updated_at
              from evidence_matrix_cells order by id
