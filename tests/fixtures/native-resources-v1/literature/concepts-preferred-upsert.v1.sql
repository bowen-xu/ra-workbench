insert into concept_terms (concept_id, term_id, relation_kind, created_at)
             values (?1, ?2, 'preferred', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
             on conflict(concept_id, term_id) do update set relation_kind = 'preferred'