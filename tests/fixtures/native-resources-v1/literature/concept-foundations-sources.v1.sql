select assertion.paper_id, concept.id, concept.revision,
                coalesce(nullif(concept.label, ''), preferred.text, '未命名概念')
         from paper_concept_assertions assertion
         join paper_concept_assertion_revisions revision
           on revision.assertion_id = assertion.id and revision.revision = assertion.current_revision
         left join term_usages usage on usage.id = assertion.term_usage_id
         join concepts concept on concept.id = coalesce(assertion.concept_id, usage.concept_id)
         left join concept_terms preferred_link
           on preferred_link.concept_id = concept.id and preferred_link.relation_kind = 'preferred'
         left join terms preferred on preferred.id = preferred_link.term_id
         where assertion.paper_id in (select value from json_each(?1))
           and revision.paper_function = 'foundational'
         order by assertion.paper_id, concept.id
