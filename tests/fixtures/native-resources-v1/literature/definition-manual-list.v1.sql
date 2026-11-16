select id from definition_claims
             where concept_id = ?1 or concept_id in (
                 select source_concept_id from concept_identity_mappings
                 where canonical_concept_id = ?1
             )
             order by updated_at desc, id desc
