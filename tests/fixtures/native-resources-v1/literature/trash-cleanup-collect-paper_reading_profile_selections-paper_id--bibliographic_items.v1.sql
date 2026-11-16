insert or ignore into literature_cleanup_rows(item_id,table_name,row_key)
                select ?1,?2,c.rowid from "paper_reading_profile_selections" c join "bibliographic_items" p on c."paper_id"=p."id"
                join literature_cleanup_rows s on s.item_id=?1 and s.table_name=?3 and s.row_key=p.rowid