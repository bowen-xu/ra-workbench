select link.anchor_id, dc.concept_id,
                coalesce(nullif(c.label, ''), preferred.text, '未命名概念'),
                anchor.item_id, paper.title, dc.id, dc.revision, anchor.page_index,
                anchor.page_label, anchor.quote_exact, dc.statement, dc.scope_note
         from definition_claims dc
         join definition_claim_evidence_links link on link.definition_claim_id = dc.id
         join source_anchors anchor on anchor.id = link.anchor_id
         join bibliographic_items paper on paper.id = anchor.item_id
         join concepts c on c.id = dc.concept_id
         left join concept_terms preferred_link
           on preferred_link.concept_id = c.id and preferred_link.relation_kind = 'preferred'
         left join terms preferred on preferred.id = preferred_link.term_id
         where dc.concept_id in (select value from json_each(?1))
         order by dc.updated_at desc, link.anchor_id
         limit ?2