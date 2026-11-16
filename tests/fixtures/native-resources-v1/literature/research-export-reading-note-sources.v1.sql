select block_id, page_id, note_type, color, current_anchor_id, paper_id_snapshot, paper_title_snapshot,
                     attachment_id_snapshot, attachment_name_snapshot, content_sha256, page_index,
                     page_label, rects_json, quote_exact, quote_prefix, quote_suffix,
                     locator_version, selector_json, created_at, updated_at from reading_note_sources
              order by block_id
