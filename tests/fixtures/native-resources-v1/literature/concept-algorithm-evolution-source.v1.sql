select concept.id, concept.revision,
                coalesce(nullif(concept.label, ''), preferred.text, '未命名概念'),
                paper.id, paper.title, paper.issued_year, assertion.id, revision.revision,
                revision.paper_function, revision.novelty_status, revision.explanation,
                coalesce(usage.anchor_id, (
                    select anchor.id from term_usages other_usage
                    join source_anchors anchor on anchor.id = other_usage.anchor_id
                    where other_usage.concept_id = concept.id and anchor.item_id = paper.id
                    order by other_usage.updated_at desc, anchor.id limit 1
                ))
         from paper_concept_assertions assertion
         join paper_concept_assertion_revisions revision
           on revision.assertion_id = assertion.id and revision.revision = assertion.current_revision
         left join term_usages usage on usage.id = assertion.term_usage_id
         join concepts concept on concept.id = coalesce(assertion.concept_id, usage.concept_id)
         left join concept_terms preferred_link
           on preferred_link.concept_id = concept.id and preferred_link.relation_kind = 'preferred'
         left join terms preferred on preferred.id = preferred_link.term_id
         join bibliographic_items paper on paper.id = assertion.paper_id
         where concept.id = ?1
         order by coalesce(paper.issued_year, 999999), paper.id, assertion.id
