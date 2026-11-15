import { notePageId, noteBlockId } from "@research-assistant/core/notes";
import { decodeAgentReadingNoteContextPreview, decodeAgentNoteContextPreview, decodeAgentPaperContextPreview, type AgentPaperContextPreview } from "@research-assistant/core/agent";
import type { AppGateways } from "../../../core/runtime/appGatewayTypes";
import { getNotesGateway } from "../../../core/features/notes/gateways";
import { getLiteratureGateway } from "@research-assistant/literature-library-plugin/literatureGateway";
import { getDoclingGateway } from "../../../core/services/doclingGateway";
import { writingDiffDemoHash } from "../../previews/agent/WritingDiffDemoStorage";

export function createMockResearchContexts(storage: Storage) {
  const papers = new Map<string, AgentPaperContextPreview>(JSON.parse(storage.getItem("paper-contexts") ?? "[]") as [string, AgentPaperContextPreview][]);
  const note: AppGateways["agentNoteContextGateway"] = { preview: async request => {
    const snapshot = await getNotesGateway().loadDocument(notePageId(request.pageId));
    if (snapshot.page.revision !== request.expectedRevision) throw new Error("笔记已更新，请重新预览上下文");
    const blocks = request.blockId ? snapshot.document.blocks.filter(block => block.id === request.blockId) : snapshot.document.blocks;
    const text = JSON.stringify({ schemaVersion: 1, pageId: request.pageId, title: snapshot.page.title, pageRevision: snapshot.page.revision, blocks });
    const hash = await writingDiffDemoHash(text);
    return decodeAgentNoteContextPreview({ schemaVersion: 1, source: { kind: "note", resourceId: request.pageId, sourceId: `note:${hash}`, revision: `revision:${snapshot.page.revision}:sha256:${hash}`, anchor: JSON.stringify({ schemaVersion: 1, pageId: request.pageId, pageRevision: snapshot.page.revision, blockId: request.blockId ?? null }), text } }, request);
  } };
  const reading: AppGateways["agentReadingNoteContextGateway"] = { preview: async request => {
    const value = await getNotesGateway().getReadingNote(noteBlockId(request.readingNoteId));
    if (value.revision !== request.expectedRevision) throw new Error("批注已更新，请重新预览上下文");
    const source = { anchorId: value.source.anchorId ?? value.id, paperId: value.source.paperId, paperTitle: value.source.paperTitle, attachmentId: value.source.attachmentId, contentSha256: value.source.contentSha256, pageIndex: value.source.pageIndex, pageLabel: value.source.pageLabel ?? "1", rects: value.source.rects, quoteExact: value.source.quoteExact, quotePrefix: value.source.quotePrefix, quoteSuffix: value.source.quoteSuffix, selector: value.source.selector ?? null, locatorVersion: value.source.selector ? 2 : 1 };
    if (!value.pageId) throw new Error("阅读笔记未迁移");
    const snapshot = await getNotesGateway().loadDocument(value.pageId);
    if (snapshot.page.status !== "active" || snapshot.page.revision !== request.expectedRevision) throw new Error("笔记已变化");
    const content = { schemaVersion: 1, pageId: snapshot.page.id, title: snapshot.page.title, pageRevision: snapshot.page.revision, blocks: snapshot.document.blocks };
    const text = JSON.stringify({ schemaVersion: 1, readingNoteId: value.id, noteRevision: value.revision, noteType: value.noteType, content, source }), hash = await writingDiffDemoHash(text);
    const anchor = JSON.stringify({ schemaVersion: 1, readingNoteId: value.id, noteRevision: value.revision, sourceAnchorId: source.anchorId, paperId: source.paperId, attachmentId: source.attachmentId, contentSha256: source.contentSha256, pageIndex: source.pageIndex, locatorVersion: source.locatorVersion });
    return decodeAgentReadingNoteContextPreview({ schemaVersion: 1, source: { kind: "note", resourceId: value.id, sourceId: `reading-note:${value.id}`, revision: `revision:${value.revision}:sha256:${hash}`, anchor, text } }, request);
  } };
  const paper: AppGateways["agentPaperContextGateway"] = {
    prepare: async request => {
      const item = await getLiteratureGateway().get(request.paperId);
      const selected = item.attachments.filter(attachment => request.attachmentIds.includes(attachment.id));
      const attachments = await Promise.all(selected.map(async (attachment, ordinal) => {
        const runs = await getDoclingGateway().listParseRuns(item.id);
        const run = runs.find(value => value.attachmentId === attachment.id) ?? await getDoclingGateway().parseAttachment(item.id, attachment.id);
        return { attachmentId: attachment.id, contentSha256: attachment.contentSha256, parseRunId: run.id, ordinal };
      }));
      const text = `${item.title}\n${item.abstractText}`, hash = await writingDiffDemoHash(JSON.stringify({ item, attachments }));
      const value: AgentPaperContextPreview = { schemaVersion: 1, paperId: item.id, snapshotId: `paper-context-${hash}`, contextHash: hash, attachments, items: await Promise.all(attachments.map(async ({ ordinal: _ordinal, ...attachment }) => ({ sourceId: `paper-source:${await writingDiffDemoHash(JSON.stringify(attachment))}`, id: "preview-text", ...attachment, rawRef: "#/texts/0", kind: "text" as const, label: "text", pageIndex: 0, regions: [{ pageIndex: 0, rect: { x: 0.1, y: 0.05, width: 0.5, height: 0.04 } }], charspan: null, sourceTextHash: await writingDiffDemoHash(text), text, tableJson: null, imageContentSha256: null }))) };
      papers.set(value.snapshotId, value); storage.setItem("paper-contexts", JSON.stringify([...papers]));
      return { schemaVersion: 1, paperId: item.id, snapshotId: value.snapshotId, contextHash: hash };
    },
    preview: async request => { const value = papers.get(request.snapshotId); if (!value) throw new Error("模拟文献上下文不存在"); return decodeAgentPaperContextPreview(value, request); },
  };
  return { note, reading, paper };
}
