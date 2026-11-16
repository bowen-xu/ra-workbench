update reference_records set
             citing_paper_id = ?2, source_anchor_id = ?3, raw_text = ?4,
             fields_json = ?5, parse_state = ?6, identity_state = ?7,
             matched_item_id = ?8, semantic_state = ?9, usage_kind = ?10,
             usage_summary = ?11, usage_anchor_id = ?12, current_revision = ?13,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         where id = ?1 and current_revision = ?14