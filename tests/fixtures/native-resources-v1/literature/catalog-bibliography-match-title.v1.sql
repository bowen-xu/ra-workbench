select id from bibliographic_items where lower(trim(title))=lower(trim(?1))
            and archived_at is null and trashed_at is null
