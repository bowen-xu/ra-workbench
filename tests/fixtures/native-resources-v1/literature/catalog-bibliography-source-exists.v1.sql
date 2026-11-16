select exists(
                select 1 from bibliographic_items
                where id = ?1 and archived_at is null and trashed_at is null
            )
