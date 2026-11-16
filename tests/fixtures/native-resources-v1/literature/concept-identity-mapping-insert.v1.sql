insert into concept_identity_mappings
             (source_concept_id, canonical_concept_id, merge_event_id, created_at)
             values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
