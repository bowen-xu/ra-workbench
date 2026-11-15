import { createNoteDocumentAdapter, DocumentVersionConflict, HbsSession, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { directTextDiff } from "../../../core/components/hbs/codeMirrorHbsPolicy";
import { noteEditorSchema } from "../../../core/features/notes/editor/noteEditorSchema";

const inline = (text: string) => ({ kind: "inline" as const, items: [{ type: "text" as const, text, styles: [] }] });
export const rowText = (row: readonly unknown[]) => (row[0] as ReturnType<typeof inline>).items.map(item => item.text).join("");
export const tableRows = (document: NoteDocumentV1) => {
  const content = document.blocks.find(block => block.id === "table-review-grid")?.content;
  if (content?.kind !== "table") throw new Error("MODEL_TABLE_MISSING：示例表格不存在。");
  return content.rows;
};
export const withRows = (document: NoteDocumentV1, rows: ReturnType<typeof tableRows>): NoteDocumentV1 => ({ ...document,
  blocks: document.blocks.map(block => block.id === "table-review-grid" && block.content.kind === "table"
    ? { ...block, content: { ...block.content, rows } } : block),
});
const initial: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("table-review-preview"), blocks: [
  { id: noteBlockId("table-review-title"), type: "heading", props: { level: 2 }, content: inline("研究计划"), children: [] },
  { id: noteBlockId("table-review-grid"), type: "table", props: {}, children: [], content: { kind: "table", rows: [
    [inline("条件"), inline("准确率")], [inline("无噪声"), inline("0.80")], [inline("有噪声"), inline("0.65")],
  ] } },
  { id: noteBlockId("table-review-note"), type: "paragraph", props: {}, content: inline("未变化的行保持普通样式；单元格与表格继续使用当前笔记组件。"), children: [] },
] };

/** Only the document and I/O are synthetic. The real HBS session, adapter,
 * editor schema, history, review and comparison execute without overrides. */
export async function createTableScenario(mode: "combined" | "plain" | "duplicates" = "combined") {
  const adapter = createNoteDocumentAdapter(directTextDiff);
  // Mock content has the same schema defaults as an actual newly created note.
  const canonical: NoteDocumentV1 = { ...initial, blocks: initial.blocks.map(block => {
    const props = noteEditorSchema.blockSchema[block.type as keyof typeof noteEditorSchema.blockSchema].propSchema;
    return { ...block, props: { ...Object.fromEntries(Object.entries(props).map(([key, rule]) => [key, rule.default])), ...block.props } };
  }) };
  const base = mode === "duplicates" ? withRows(canonical, [[inline("条件"), inline("准确率")],
    [inline("重复"), inline("0.80")], [inline("重复"), inline("0.80")]]) : canonical;
  let server: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: base, baseline: base };
  const events: string[] = [];
  const session = await HbsSession.open({ adapter, requestId: n => `table-preview-${n}`,
    preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => server, commit: async request => {
      if (request.expectedVersion !== server.version) throw new DocumentVersionConflict(server, request.id);
      server = { version: server.version + 1, saved: request.saved, baseline: request.baseline };
      events.push(`内存网关提交 ${request.id} · v${server.version}`);
      return server;
    } },
  });
  const external = async (document: NoteDocumentV1, label: string) => {
    server = { ...server, version: server.version + 1, saved: document };
    events.push(`外部修改：${label} · v${server.version}`);
    await session.syncLatest();
  };
  let accuracy = 44;
  const columnIndex = (label: string) => tableRows(server.saved)[0]!.findIndex(cell => rowText([cell]) === label);
  const editCell = () => {
    const next = `0.${accuracy++}`;
    return external(withRows(server.saved, tableRows(server.saved).map(row => rowText([row[columnIndex("条件")]!]) === "无噪声"
      ? row.map((cell, column) => column === columnIndex("准确率") ? inline(next) : cell) : row)), `无噪声准确率 → ${next}`);
  };
  const insertRow = () => {
    const rows = [...tableRows(server.saved)];
    rows.splice(1, 0, rows[0]!.map((_cell, column) => inline(column === columnIndex("条件") ? `新增条件 ${server.version}`
      : column === columnIndex("准确率") ? "0.50" : `新增行／列 ${column}`)));
    return external(withRows(server.saved, rows), "在第一条数据前插入行");
  };
  const insertColumn = (position = 1) => external(withRows(server.saved, tableRows(server.saved).map((row, index) => {
    const cells = [...row]; cells.splice(position, 0, inline(index === 0 ? `新增列 ${server.version}` : `列 ${server.version}／行 ${index}`));
    return cells;
  })), "插入列");
  if (mode === "combined") { await editCell(); await insertRow(); }
  if (mode === "duplicates") await external(withRows(base, [...tableRows(base), [inline("重复"), inline("0.80")]]), "增加相同内容的行");
  return { session, events, editCell, insertRow, insertColumn,
    deleteRow: () => external(withRows(server.saved, tableRows(server.saved).filter(row => rowText([row[columnIndex("条件")]!]) !== "有噪声")), "删除有噪声行") };
}
