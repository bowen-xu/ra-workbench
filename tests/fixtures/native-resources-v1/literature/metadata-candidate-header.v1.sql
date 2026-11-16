select draft_json, item_id, source_kind, source_reference, import_result_id
             from metadata_candidates where id = ?1 and state = 'pending'