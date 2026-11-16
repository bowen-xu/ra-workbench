update metadata_provenance
         set confirmed_by_user = ?4, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where item_id = ?1 and field_name = ?2 and confirmed_by_user = 1 and updated_at = ?3
