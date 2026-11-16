select d.id, d.concept_id, d.statement, d.scope_note, d.provenance_kind,
                    d.epistemic_role, d.revision,
                    (select count(*) from definition_claim_evidence_links e where e.definition_claim_id = d.id),
                    d.created_at, d.updated_at
             from definition_claims d where d.id = ?1
