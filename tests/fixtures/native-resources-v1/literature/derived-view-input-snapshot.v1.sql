select r.snapshot_json from evidence_matrix_cell_revisions r
                 join evidence_matrix_cells c on c.id = r.cell_id
                 join evidence_matrices m on m.id = c.matrix_id
                 where c.id = ?1 and r.revision = ?2 and m.project_id = ?3
