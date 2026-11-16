select context_item_id, attachment_id, content_sha256, parse_run_id, raw_ref,
                item_kind, label, page_index, regions_json, charspan_json, source_text_hash,
                text_content, table_json, image_content_sha256
         from paper_context_items where snapshot_id = ?1 order by context_item_id