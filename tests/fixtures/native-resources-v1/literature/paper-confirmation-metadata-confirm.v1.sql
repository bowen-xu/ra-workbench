update metadata_provenance set confirmed_by_user = 1, updated_at = ?4
         where item_id = ?1 and field_name = ?2 and confirmed_by_user = 0 and updated_at = ?3
