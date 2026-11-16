insert into metadata_candidates
            (id, item_id, source_kind, source_reference, draft_json, raw_payload,
             import_result_id, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))