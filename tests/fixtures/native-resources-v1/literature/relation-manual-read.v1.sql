select r.id, r.project_id, p.title, r.subject_kind, r.subject_id,
                    r.subject_label_snapshot, r.predicate_kind, r.object_kind, r.object_id,
                    r.object_label_snapshot, r.scope_note, r.provenance_kind, r.epistemic_role,
                    r.revision, (select count(*) from relation_assertion_evidence_links e
                                 where e.relation_assertion_id = r.id),
                    r.created_at, r.updated_at
             from relation_assertions r left join research_projects p on p.id = r.project_id
             where r.id = ?1
