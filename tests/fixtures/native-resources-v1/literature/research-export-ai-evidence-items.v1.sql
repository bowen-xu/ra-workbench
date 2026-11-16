select id, bundle_id, citation_key, ordinal, anchor_id, paper_id, paper_title,
                     attachment_id, content_sha256, page_index, page_label, quote_exact,
                     quote_prefix, quote_suffix, context_text, relation_kind, validation_status,
                     created_at from ai_evidence_items order by bundle_id, ordinal
