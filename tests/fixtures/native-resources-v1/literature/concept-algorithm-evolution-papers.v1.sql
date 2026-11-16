select distinct assertion.paper_id
         from paper_concept_assertions assertion
         left join term_usages usage on usage.id = assertion.term_usage_id
         where coalesce(assertion.concept_id, usage.concept_id) = ?1
         order by assertion.paper_id
