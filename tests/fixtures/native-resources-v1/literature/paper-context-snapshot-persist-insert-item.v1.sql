insert into paper_context_items
             (snapshot_id, context_item_id, attachment_id, content_sha256, parse_run_id,
              raw_ref, item_kind, label, page_index, regions_json, charspan_json,
              source_text_hash, text_content, table_json, image_content_sha256, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     ?15, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))