select ct.term_id,
                    case when exists(
                        select 1 from concept_terms own
                        where own.concept_id = ?1 and own.term_id = ct.term_id
                          and own.relation_kind = 'preferred'
                    ) then 'preferred' else 'alias' end as resolved_relation
             from concept_terms ct
             where ct.concept_id = ?1 or ct.concept_id in (
                 select source_concept_id from concept_identity_mappings
                 where canonical_concept_id = ?1
             )
             group by ct.term_id
             order by case resolved_relation when 'preferred' then 0 else 1 end,
                      min(ct.created_at), ct.term_id