import { noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
export const textFixture = "研究计划\n\n样本量为 120。\n模型准确率为 80%。\n\n下一步：补充复杂场景的验证。";
const inline = (text: string) => ({ kind: "inline" as const, items: [{ type: "text" as const, text, styles: [] }] });
const block = (id: string, text: string): NoteBlockV1 => ({ id: noteBlockId(id), type: "paragraph", props: { textColor: "default", backgroundColor: "default", textAlignment: "left" }, content: inline(text), children: [] });
export const noteFixture: NoteDocumentV1 = {
  schemaVersion: 1, pageId: notePageId("00000000-0000-4000-8000-000000000186"), blocks: [
    { ...block("hbs-title", "研究计划"), type: "heading", props: { ...block("hbs-title", "").props, level: 2, isToggleable: false } },
    block("hbs-score", "模型准确率为 80%。"),
    { ...block("hbs-method", "评估方法"), children: [block("hbs-nested-a", "重复实验 3 次。"), block("hbs-nested-b", "记录每次实验的误差。")] },
    { ...block("hbs-table", ""), type: "table", content: { kind: "table", rows: [
      [inline("条件"), inline("准确率")], [inline("无噪声"), inline("0.80")], [inline("有噪声"), inline("0.65")],
    ] } },
    { ...block("hbs-formula", ""), type: "formulaBlock", props: {}, content: { kind: "formula", source: "E = mc^2" } },
    { ...block("hbs-inline", ""), content: { kind: "inline", items: [{ type: "text", text: "行内公式：", styles: [] }, { type: "inlineFormula", source: "x^2" }] } },
    { ...block("hbs-mention", ""), content: { kind: "inline", items: [{ type: "mention", targetKind: "paper", targetId: "synthetic-paper", labelSnapshot: "合成文献提及" }] } },
    block("hbs-next", "下一步：补充复杂场景的验证。"),
  ],
};
export const structuredFixture = {
  kind: "paper_overview_unit_v1", schemaVersion: 1, profileId: "generic_overview_v1",
  fields: ["research_question", "approach", "key_result", "conclusion", "limitation", "open_question"].map((field, index) => ({ field,
    status: index < 2 ? "filled" : "not_found", statements: index < 2 ? [{ id: `hbs-statement-${index}`,
      text: index ? "比较不同噪声条件下的准确率。" : "局部观察如何支持逻辑推理？", epistemicRole: "uncertain", evidenceItemIds: [] }] : [] })),
};
