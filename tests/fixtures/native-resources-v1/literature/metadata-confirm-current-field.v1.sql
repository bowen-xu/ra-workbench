update metadata_provenance
                 set confirmed_by_user = 1,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                 where item_id = ?1 and field_name = ?2