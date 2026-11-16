select c.id, coalesce(nullif(c.label, ''), t.text), c.revision,
                (select count(*) from concept_terms ct2 where ct2.concept_id = c.id),
                (select count(*) from definition_claims d where d.concept_id = c.id),
                (select count(distinct u.anchor_id) from term_usages u where u.concept_id = c.id)
         from concepts c join concept_terms ct on ct.concept_id = c.id
              and ct.relation_kind = 'preferred'
         join terms t on t.id = ct.term_id where c.id = ?1
