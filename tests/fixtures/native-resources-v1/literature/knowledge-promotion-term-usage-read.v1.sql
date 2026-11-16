select u.id, u.term_id, u.concept_id, u.anchor_id, a.item_id, b.title,
                    a.attachment_id, a.content_sha256, a.page_index, a.page_label,
                    a.quote_exact, u.usage_text, u.provenance_kind, u.revision,
                    u.created_at, u.updated_at
             from term_usages u
             join source_anchors a on a.id = u.anchor_id
             join bibliographic_items b on b.id = a.item_id
             where u.id = ?1
