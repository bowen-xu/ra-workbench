select distinct c.id, coalesce(nullif(c.label, ''), preferred.text),
                trim(c.domain || case when c.disambiguation = '' then '' else ' · ' || c.disambiguation end)
         from concepts c
         join concept_terms preferred_ct on preferred_ct.concept_id = c.id
              and preferred_ct.relation_kind = 'preferred'
         join terms preferred on preferred.id = preferred_ct.term_id
         left join concept_terms ct on ct.concept_id = c.id or ct.concept_id in (
             select source_concept_id from concept_identity_mappings
             where canonical_concept_id = c.id
         )
         left join terms t on t.id = ct.term_id
         where c.deleted_at is null and not exists(select 1 from concept_identity_mappings m
                          where m.source_concept_id = c.id)
           and (?1 = '' or instr(lower(c.label || ' ' || c.explanation || ' ' || c.disambiguation || ' ' || t.text), lower(?1)) > 0)
         order by c.updated_at desc limit 20
