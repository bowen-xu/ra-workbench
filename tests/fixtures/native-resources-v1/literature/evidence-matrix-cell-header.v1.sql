select id, current_revision from evidence_matrix_cells
             where matrix_id = ?1 and paper_id = ?2 and dimension_id = ?3
