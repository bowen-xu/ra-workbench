delete from agent_paper_submission_scopes where rowid in
            (select row_key from literature_cleanup_rows where item_id=?1 and table_name=?2)