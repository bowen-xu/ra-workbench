select c.id, coalesce(nullif(c.label, ''), t.text),
                        trim(c.domain || case when c.disambiguation = '' then '' else ' · ' || c.disambiguation end)
                 from concepts c join concept_terms ct on ct.concept_id = c.id and ct.relation_kind = 'preferred'
                 join terms t on t.id = ct.term_id where c.id = ?1 and c.deleted_at is null
                 and not exists(select 1 from concept_identity_mappings m
                                where m.source_concept_id = c.id)
