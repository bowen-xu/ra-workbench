select id from bibliographic_items
             where issued_year = ?1 and archived_at is null and trashed_at is null