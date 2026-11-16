select id, citing_paper_id, source_anchor_id, raw_text, fields_json,
                    parse_state, identity_state, matched_item_id, semantic_state,
                    usage_kind, usage_summary, usage_anchor_id, provenance_kind,
                    current_revision, created_at, updated_at
             from reference_records where id = ?1