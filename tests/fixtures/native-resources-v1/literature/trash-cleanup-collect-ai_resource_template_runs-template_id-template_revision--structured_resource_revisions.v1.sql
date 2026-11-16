insert or ignore into literature_cleanup_rows(item_id,table_name,row_key)
                select ?1,?2,c.rowid from "ai_resource_template_runs" c join "structured_resource_revisions" p on c."template_id"=p."resource_id" and c."template_revision"=p."revision"
                join literature_cleanup_rows s on s.item_id=?1 and s.table_name=?3 and s.row_key=p.rowid