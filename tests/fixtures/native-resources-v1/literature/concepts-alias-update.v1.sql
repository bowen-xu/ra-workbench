update concept_terms set relation_kind = 'alias'
             where concept_id = ?1 and relation_kind = 'preferred' and term_id <> ?2