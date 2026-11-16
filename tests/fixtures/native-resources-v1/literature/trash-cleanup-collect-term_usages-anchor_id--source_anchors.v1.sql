insert or ignore into literature_cleanup_rows(item_id,table_name,row_key)
                select ?1,?2,c.rowid from "term_usages" c join "source_anchors" p on c."anchor_id"=p."id"
                join literature_cleanup_rows s on s.item_id=?1 and s.table_name=?3 and s.row_key=p.rowid