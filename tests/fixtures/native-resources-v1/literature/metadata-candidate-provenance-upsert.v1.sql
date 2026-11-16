insert into metadata_provenance
                (item_id, field_name, source_kind, source_reference, confirmed_by_user, updated_at)
             values (?1, ?2, ?3, ?4, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(item_id, field_name) do update set
                source_kind = excluded.source_kind,
                source_reference = excluded.source_reference,
                confirmed_by_user = 1,
                updated_at = excluded.updated_at