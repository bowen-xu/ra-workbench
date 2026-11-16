update bibliographic_items set archived_at = null,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and archived_at is not null and trashed_at is null