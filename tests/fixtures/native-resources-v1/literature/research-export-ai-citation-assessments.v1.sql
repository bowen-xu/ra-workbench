select id, candidate_id, candidate_revision, statement_id, evidence_item_id,
                     assessment_revision, semantic_relation, rationale, idempotency_key,
                     request_hash, assessed_at from ai_citation_assessments
              order by candidate_id, candidate_revision, statement_id, evidence_item_id,
                       assessment_revision
