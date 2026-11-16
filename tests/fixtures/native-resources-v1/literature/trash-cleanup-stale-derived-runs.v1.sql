update concept_algorithm_runs set status='stale',error_code='source_paper_deleted',
        finished_at=coalesce(finished_at,strftime('%Y-%m-%dT%H:%M:%fZ','now')),
        result_count=(select count(*) from concept_evolution_results e where e.run_id=concept_algorithm_runs.id
            and not exists(select 1 from literature_cleanup_rows s where s.item_id=?1 and s.table_name='concept_evolution_results' and s.row_key=e.rowid))
          +(select count(*) from paper_foundation_similarity_results e where e.run_id=concept_algorithm_runs.id
            and not exists(select 1 from literature_cleanup_rows s where s.item_id=?1 and s.table_name='paper_foundation_similarity_results' and s.row_key=e.rowid))
        where exists(select 1 from json_each(paper_scope_json) where type='text' and value=?1)
          or id in(select e.run_id from concept_evolution_results e join literature_cleanup_rows s on s.item_id=?1 and s.table_name='concept_evolution_results' and s.row_key=e.rowid)
          or id in(select e.run_id from paper_foundation_similarity_results e join literature_cleanup_rows s on s.item_id=?1 and s.table_name='paper_foundation_similarity_results' and s.row_key=e.rowid)