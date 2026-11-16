insert into metadata_provenance
                (item_id, field_name, source_kind, source_reference, confirmed_by_user, updated_at)
             select ?2, field_name, source_kind, source_reference, confirmed_by_user, updated_at
             from metadata_provenance where item_id = ?1 and field_name = ?3