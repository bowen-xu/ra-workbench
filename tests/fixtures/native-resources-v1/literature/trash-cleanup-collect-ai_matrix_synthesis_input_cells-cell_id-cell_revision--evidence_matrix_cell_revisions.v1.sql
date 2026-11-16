insert or ignore into literature_cleanup_rows(item_id,table_name,row_key)
                select ?1,?2,c.rowid from "ai_matrix_synthesis_input_cells" c join "evidence_matrix_cell_revisions" p on c."cell_id"=p."cell_id" and c."cell_revision"=p."revision"
                join literature_cleanup_rows s on s.item_id=?1 and s.table_name=?3 and s.row_key=p.rowid