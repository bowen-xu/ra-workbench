update reading_note_sources
         set paper_id_snapshot = ?1,
             paper_title_snapshot = (select title from bibliographic_items where id = ?1),
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where current_anchor_id in (select value from json_each(?2))