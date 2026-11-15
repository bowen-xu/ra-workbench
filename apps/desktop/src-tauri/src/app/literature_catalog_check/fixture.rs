use crate::storage::AppStore;
use serde_json::{Value, json};

pub(super) fn seed(store: &AppStore) -> Result<(), crate::storage::StorageError> {
    let db = store.connection()?;
    db.execute_batch("insert into bibliographic_items(id,item_type,title,abstract_text,issued_year,
        container_title,conference_name,proceedings_title,place,volume,issue,pages,url,language,
        bibtex_key,raw_bibtex,reading_status,created_at,updated_at)
        values('paper-new','journalArticle','Needle 新题录','原摘要',2025,'刊物','会议','论文集',
        '地点','v','i','1-2','https://example.invalid/paper','zh','key',null,'reading','2025-02-01','2025-02-02');
        insert into bibliographic_items(id,item_type,title,created_at,updated_at)
        values('paper-old','book','Older','2025-01-01','2025-01-01');
        insert into bibliographic_items(id,item_type,title,created_at,updated_at,trashed_at)
        values('paper-trashed','book','Trashed','2025-03-01','2025-03-01','2025-03-02');
        insert into bibliographic_items(id,item_type,title,created_at,updated_at,archived_at)
        values('paper-archived','book','Archived','2025-04-01','2025-04-01','2025-04-02');
        insert into creators(id,family_name,given_name,literal_name) values('creator','Family','Given','');
        insert into item_creators(item_id,creator_id,role,position) values('paper-new','creator','author',0);
        insert into tags(id,name) values('tag-z','zeta'),('tag-a','Alpha');
        insert into item_tags(item_id,tag_id) values('paper-new','tag-z'),('paper-new','tag-a');
        insert into identifiers(id,item_id,kind,normalized_value,display_value) values('identifier','paper-new','doi','10.0/synthetic','10.0/Synthetic');
        insert into research_projects(id,title,summary,created_at) values('project','Project','','2025-01-01');
        insert into project_items(item_id,project_id,added_at) values('paper-new','project','2025-01-01');
        insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at)
        values('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','objects/a','application/pdf',17,'2025-01-01');
        insert into attachments(id,item_id,content_sha256,display_name,created_at)
        values('attachment','paper-new','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','paper.pdf','2025-01-01');
        insert into metadata_provenance(item_id,field_name,source_kind,confirmed_by_user,updated_at)
        values('paper-new','title','manual',1,'2025-01-01');
        insert into literature_search(item_id,title,creators,identifiers,tags,full_text)
        values('paper-new','Needle 新题录','Family Given','10.0/Synthetic','Alpha zeta',''),
        ('paper-old','Older','','','','');")?;
    Ok(())
}

pub(super) fn expected() -> Value {
    json!({"id":"paper-new","itemType":"journalArticle","title":"Needle 新题录",
        "abstractText":"原摘要","issuedYear":2025,"containerTitle":"刊物","conferenceName":"会议",
        "proceedingsTitle":"论文集","place":"地点","volume":"v","issue":"i","pages":"1-2",
        "doi":"10.0/Synthetic","url":"https://example.invalid/paper","language":"zh",
        "bibtexKey":"key","rawBibtex":null,"readingStatus":"reading",
        "creators":[{"familyName":"Family","givenName":"Given","literalName":"","role":"author"}],
        "tags":["Alpha","zeta"],"attachments":[{"id":"attachment","displayName":"paper.pdf",
            "mimeType":"application/pdf","byteSize":17,"contentSha256":"a".repeat(64)}],
        "projectIds":["project"],"metadataSources":[{"fieldName":"title","sourceKind":"manual","confirmedByUser":true}],
        "createdAt":"2025-02-01","updatedAt":"2025-02-02"})
}
