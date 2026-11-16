update bibliographic_items
         set trashed_at = null, archived_at = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and trashed_at is not null