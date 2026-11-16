select id from bibliographic_items
         where id <> ?1 and lower(trim(title)) = ?2 and issued_year is ?3
           and archived_at is null and trashed_at is null