select usage.anchor_id, usage.concept_id,
                coalesce(nullif(c.label, ''), preferred.text, '未命名概念'),
                anchor.item_id, paper.title, usage.id, usage.revision, anchor.page_index,
                anchor.page_label, anchor.quote_exact, usage.usage_text, term.text
         from term_usages usage
         join terms term on term.id = usage.term_id
         join source_anchors anchor on anchor.id = usage.anchor_id
         join bibliographic_items paper on paper.id = anchor.item_id
         join concepts c on c.id = usage.concept_id
         left join concept_terms preferred_link
           on preferred_link.concept_id = c.id and preferred_link.relation_kind = 'preferred'
         left join terms preferred on preferred.id = preferred_link.term_id
         where usage.concept_id in (select value from json_each(?1))
         order by usage.updated_at desc, usage.anchor_id
         limit ?2