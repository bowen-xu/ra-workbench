update ai_runs set context_snapshot_id=null where context_snapshot_id in
            (select p.id from paper_context_snapshots p join literature_cleanup_rows s
             on s.item_id=?1 and s.table_name=?2 and s.row_key=p.rowid)