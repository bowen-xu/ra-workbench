import type {AgentContextPart, AgentContextSelection} from "@research-assistant/core/agent";

/** Synthetic inventory only; the context dialog, permissions and pagination are production UI. */
export function writingDemoContextCatalog(manuscriptId: string, paths: readonly string[]): readonly AgentContextSelection[] {
  const part = (resource: AgentContextPart["resource"], label: string): AgentContextPart => ({resource, label, selected: true, access: resource.kind === "paper" ? "readOnly" : "readWrite"});
  const rows: AgentContextSelection[] = [{label: "当前演示稿件", resource: {
    kind: "manuscript", resourceId: manuscriptId, newMemberAccess: "readWrite", groupAccess: "readWrite",
    parts: paths.map(path => part({kind: "manuscriptFile", resourceId: manuscriptId, path}, path)),
  }}];
  const topics = ["研究方法与可复现性", "证据与论证", "文献综述", "模型与评估", "学术写作", "开放研究"];
  for (let index = 1; index <= 24; index++) {
    const id = `10000000-0000-4000-8000-${String(index).padStart(12, "0")}`;
    rows.push({label: `模拟文献 ${String(index).padStart(2, "0")} · ${topics[(index - 1) % topics.length]}`, resource: {
      kind: "paperGroup", resourceId: id, newMemberAccess: "readWrite", groupAccess: "readWrite", parts: [
        part({kind: "paperMetadata", resourceId: id}, "元数据"),
        part({kind: "paper", resourceId: id, attachmentId: `20000000-0000-4000-8000-${String(index).padStart(12, "0")}`}, "论文全文.pdf"),
        part({kind: "paperNotes", resourceId: id, excludedNoteIds: []}, "阅读笔记"),
        part({kind: "paperPortrait", resourceId: id}, "文献画像"),
      ],
    }});
  }
  for (let index = 1; index <= 12; index++) {
    const id = `30000000-0000-4000-8000-${String(index).padStart(12, "0")}`;
    rows.push({label: `模拟笔记 ${String(index).padStart(2, "0")} · ${topics[(index - 1) % topics.length]}`, resource: {
      kind: "noteGroup", resourceId: id, newMemberAccess: "readWrite", groupAccess: "readWrite", parts: [part({kind: "note", resourceId: id}, "正文")],
    }});
  }
  return rows;
}
