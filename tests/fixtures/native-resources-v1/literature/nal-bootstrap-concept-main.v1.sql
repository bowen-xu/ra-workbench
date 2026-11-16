select label, explanation, disambiguation, domain, tradition, period,
                    provenance_kind, revision,
                    (select count(*) from definition_claims d
                     where d.concept_id = c.id or d.concept_id in (
                         select source_concept_id from concept_identity_mappings
                         where canonical_concept_id = c.id
                     )),
                    (select count(distinct u.anchor_id) from term_usages u
                     where u.concept_id = c.id or u.concept_id in (
                         select source_concept_id from concept_identity_mappings
                         where canonical_concept_id = c.id
                     )),
                    created_at, updated_at
             from concepts c where id = ?1 and c.deleted_at is null