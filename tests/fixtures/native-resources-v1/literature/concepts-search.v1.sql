select distinct c.id from concepts c
             left join concept_terms ct on ct.concept_id = c.id or ct.concept_id in (
                 select source_concept_id from concept_identity_mappings
                 where canonical_concept_id = c.id
             )
             left join terms t on t.id = ct.term_id
             where c.deleted_at is null and not exists(select 1 from concept_identity_mappings m
                              where m.source_concept_id = c.id)
               and (?1 = '' or lower(c.label) like ?2 or lower(c.explanation) like ?2
                or lower(c.disambiguation) like ?2 or lower(c.domain) like ?2
                or t.normalized_text like ?2)
             order by c.updated_at desc, c.id desc limit 50