update structured_resources set template_id=null,template_revision=null where template_id in
        (select p.id from structured_resources p join literature_cleanup_rows s
         on s.item_id=?1 and s.table_name='structured_resources' and s.row_key=p.rowid)