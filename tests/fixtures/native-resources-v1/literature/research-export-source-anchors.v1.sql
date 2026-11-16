select id, item_id, attachment_id, content_sha256, selector_version, page_index,
                     page_label, rects_json, quote_exact, quote_prefix, quote_suffix, created_at
              from source_anchors order by id
