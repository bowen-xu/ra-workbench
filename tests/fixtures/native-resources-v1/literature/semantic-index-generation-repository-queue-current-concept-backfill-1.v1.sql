select sr.id from semantic_representations sr
             where sr.namespace = 'accepted_concepts'
               and sr.representation_kind = 'concept_identity'
               and not exists (
                   select 1 from semantic_representations newer
                   where newer.subject_kind = sr.subject_kind
                     and newer.subject_id = sr.subject_id
                     and newer.representation_kind = sr.representation_kind
                     and newer.subject_revision > sr.subject_revision
               )
               and not exists (
                   select 1 from embedding_records r
                   where r.representation_id = sr.id and r.index_generation_id = ?1
               )
             order by sr.created_at, sr.id