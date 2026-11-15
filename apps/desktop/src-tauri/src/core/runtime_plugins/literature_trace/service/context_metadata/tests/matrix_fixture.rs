//! Shared synthetic matrix data, not a second implementation.
use super::Fixture;
use research_workbench_native_sdk::evidence_matrix::{
    EvidenceMatrixCellDraft, EvidenceMatrixDraft, EvidenceMatrixStatement,
};
use serde_json::json;
pub(super) fn seed(f: &Fixture) {
    f.store.connection().unwrap().execute_batch("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('matrix-second','journalArticle','Synthetic comparison','now','now');
    insert into research_projects(id,title,summary,created_at) values('matrix-project','Synthetic matrix','','now');
    insert into project_items(project_id,item_id,added_at) values('matrix-project','paper','now'),('matrix-project','matrix-second','now');
    insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values('eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','ee/synthetic-matrix.pdf','application/pdf',10,'now');
    insert into attachments(id,item_id,content_sha256,display_name,created_at) values('matrix-first-pdf','paper','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','synthetic.pdf','now'),('matrix-second-pdf','matrix-second','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee','synthetic.pdf','now');
    insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,quote_exact,created_at) values('matrix-first-anchor','paper','matrix-first-pdf','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',0,'Synthetic first result','now'),('matrix-second-anchor','matrix-second','matrix-second-pdf','eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee',0,'Synthetic second result','now');").unwrap();
}
pub(super) fn draft() -> EvidenceMatrixDraft {
    serde_json::from_value(json!({"projectId":"matrix-project","title":" Compare Ω ","paperIds":["paper","matrix-second"],"dimensions":[{"id":"claim","label":"Claims"},{"id":"method","label":"Methods"}],"changeReason":" manual_matrix "})).unwrap()
}
pub(super) fn cell(id: &str) -> EvidenceMatrixCellDraft {
    serde_json::from_value(json!({"matrixId":id,"paperId":"paper","dimensionId":"claim","coverageState":"unreviewed","changeReason":" manual_cell "})).unwrap()
}
pub(super) fn statement() -> EvidenceMatrixStatement {
    EvidenceMatrixStatement {
        id: "statement-one".into(),
        text: " First result Ω ".into(),
        epistemic_role: "source_paraphrase".into(),
        evidence_anchor_ids: vec!["matrix-first-anchor".into()],
    }
}
