with configured(region,model_id) as (
 select data_text_trim_v1(?1),data_text_trim_v1(?2) where ?1 is not null and ?2 is not null
) select 'space-'||substr(data_sha256_v1('qwen-embedding|'||region||'|'||model_id||
 '|1024|float32|cosine|l2|document|query|concept-identity-v1'),1,32),
 region,model_id,1024,'concept-identity-v1','accepted_concepts' from configured