insert into concept_identity_events
         (id, operation, survivor_concept_id, source_concept_ids_json, snapshot_json,
          rationale, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
