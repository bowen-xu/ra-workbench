select c.item_id from browser_capture_items c join bibliographic_items b on b.id = c.item_id
         where c.scope = ?1 and c.session_id = ?2 and c.external_id = ?3
           and b.archived_at is null and b.trashed_at is null
