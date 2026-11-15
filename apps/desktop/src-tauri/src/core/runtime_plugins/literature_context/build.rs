use super::*;
use research_workbench_native_sdk::paper_context::{ContextAttachmentV1, ContextParseRunV1};
impl PaperContextResources {
    pub(crate) fn capture_build(
        &self,
        db: &Connection,
        paper: &str,
        attachment_ids: &[String],
    ) -> ContextResult<BuildCapture> {
        let exists = self.read(
            db,
            R::AttachmentsPaperExists,
            vec![text(paper)],
            P::FirstOptional,
        )?;
        if !matches!(exists.rows.first().map(Vec::as_slice),Some([V::Integer(value)]) if *value!=0)
        {
            return Err(failure::invalid("找不到当前文献"));
        }
        let mut resolved = Vec::new();
        if attachment_ids.is_empty() {
            resolved = self
                .read(db, R::AttachmentsAll, vec![text(paper)], P::All)?
                .rows;
        } else {
            for id in attachment_ids {
                let row = self
                    .read(
                        db,
                        R::AttachmentsExact,
                        vec![text(id), text(paper)],
                        P::FirstOptional,
                    )?
                    .rows
                    .into_iter()
                    .next()
                    .ok_or_else(|| failure::invalid("来源 PDF 不属于当前文献"))?;
                resolved.push(row);
            }
        }
        if resolved.is_empty() {
            return Err(failure::invalid("当前文献没有可用于 AI 阅读的 PDF 附件"));
        }
        let facts = PaperReadingFacts::new(db);
        let mut attachments = Vec::new();
        let mut parse_sources = Vec::new();
        for row in &resolved {
            let [V::Text(id), V::Text(name), V::Text(sha)] = row.as_slice() else {
                return Err(self.shape(R::AttachmentsAll));
            };
            let parsed = facts
                .latest_parse_run_with_source(id, sha)
                .map_err(|e| failure::ai(e.into_ai_error()))?;
            let stale = if parsed.is_none() {
                facts
                    .has_stale_parse_run(id)
                    .map_err(|e| failure::ai(e.into_ai_error()))?
            } else {
                false
            };
            parse_sources.push(parsed.as_ref().map(|(_, raw)| raw.clone()));
            attachments.push(ContextAttachmentV1 {
                id: id.clone(),
                display_name: name.clone(),
                content_sha256: sha.clone(),
                parse: parsed.map(|(record, _)| ContextParseRunV1 {
                    id: record.id,
                    json_content: record.json_content,
                }),
                has_stale_parse: stale,
            });
        }
        Ok(BuildCapture {
            input: PaperContextBuildInputV1 {
                paper_id: paper.into(),
                attachments,
            },
            witness: BuildWitness {
                resolved,
                parse_sources,
            },
        })
    }
}
