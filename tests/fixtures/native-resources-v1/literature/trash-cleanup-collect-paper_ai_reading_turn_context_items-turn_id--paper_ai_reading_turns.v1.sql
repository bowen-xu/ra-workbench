insert or ignore into literature_cleanup_rows(item_id,table_name,row_key)
                select ?1,?2,c.rowid from "paper_ai_reading_turn_context_items" c join "paper_ai_reading_turns" p on c."turn_id"=p."id"
                join literature_cleanup_rows s on s.item_id=?1 and s.table_name=?3 and s.row_key=p.rowid