update structured_reading_units
                 set revision = ?2, reverted_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 where id = ?1 and revision = ?3 and reverted_at is null
