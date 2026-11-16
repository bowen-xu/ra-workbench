select id, matrix_id, paper_id, dimension_id, coverage_state,
                    coverage_note, statements_json, provenance_kind, created_at, updated_at
             from evidence_matrix_cells where id = ?1 and current_revision = ?2
