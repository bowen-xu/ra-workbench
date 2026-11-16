update attachments set display_name=?3 where id=?2 and item_id=?1
         and exists(select 1 from bibliographic_items where id=?1 and archived_at is null and trashed_at is null)