insert into reference_records
         (id, citing_paper_id, source_anchor_id, raw_text, fields_json, parse_state,
          identity_state, matched_item_id, semantic_state, usage_kind, usage_summary,
          usage_anchor_id, provenance_kind, current_revision, created_at, updated_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, 'unverified', null, 'unreviewed', null,
                 '', null, ?7, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
