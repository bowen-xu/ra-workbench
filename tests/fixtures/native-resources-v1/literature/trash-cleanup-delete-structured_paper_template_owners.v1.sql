delete from structured_paper_template_owners where rowid in
            (select row_key from literature_cleanup_rows where item_id=?1 and table_name=?2)