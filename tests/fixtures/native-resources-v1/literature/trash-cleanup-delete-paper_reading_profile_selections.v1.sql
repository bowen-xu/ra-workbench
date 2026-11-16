delete from paper_reading_profile_selections where rowid in
            (select row_key from literature_cleanup_rows where item_id=?1 and table_name=?2)