select u.id, u.project_id, p.title, u.paper_id, b.title, u.unit_kind,
                    u.title, u.body, u.provenance_kind, u.epistemic_role, u.revision,
                    (select count(*) from reading_unit_evidence_links e where e.reading_unit_id = u.id),
                    u.created_at, u.updated_at
             from structured_reading_units u
             left join research_projects p on p.id = u.project_id
             left join bibliographic_items b on b.id = u.paper_id
             where u.id = ?1 and u.reverted_at is null