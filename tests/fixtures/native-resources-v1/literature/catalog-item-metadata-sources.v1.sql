select field_name, source_kind, confirmed_by_user from metadata_provenance
             where item_id = ?1 order by field_name