select c.id, c.project_id, p.title, c.statement, c.origin,
                c.provenance_kind, c.epistemic_role, c.revision,
                (select count(*) from evidence_links e where e.claim_id = c.id),
                c.created_at, c.updated_at
         from claims c left join research_projects p on p.id = c.project_id
         where (?1 is null or c.project_id = ?1) order by c.updated_at desc, c.id desc
