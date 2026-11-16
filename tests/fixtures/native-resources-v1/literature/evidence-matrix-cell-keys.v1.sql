select id, current_revision from evidence_matrix_cells
         where matrix_id = ?1 order by paper_id, dimension_id
