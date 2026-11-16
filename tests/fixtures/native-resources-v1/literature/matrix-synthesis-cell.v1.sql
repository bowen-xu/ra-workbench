select c.paper_id, r.snapshot_json
                 from evidence_matrix_cells c
                 join evidence_matrix_cell_revisions r
                   on r.cell_id = c.id and r.revision = ?2
                 where c.id = ?1 and c.matrix_id = ?3
