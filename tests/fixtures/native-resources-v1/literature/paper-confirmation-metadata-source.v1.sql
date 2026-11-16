select confirmed_by_user, updated_at from metadata_provenance
             where item_id = ?1 and field_name = ?2
