insert into ai_docling_evidence_context_items
             (evidence_item_id, context_item_id, parse_run_id, raw_ref, item_kind, label,
              regions_json, charspan_json, source_text_hash, image_content_sha256, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))