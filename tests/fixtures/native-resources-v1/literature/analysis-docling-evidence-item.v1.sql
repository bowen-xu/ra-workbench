insert into ai_evidence_items
             (id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title,
              attachment_id, content_sha256, page_index, page_label, quote_exact,
              quote_prefix, quote_suffix, context_text, relation_kind, validation_status, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '', '',
                     ?13, 'direct_quote', 'valid', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))