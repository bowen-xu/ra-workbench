import {withSourceNamespaces} from "./sourceDirectoryFixtures";
// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {AgentEvidenceInput} from "@research-assistant/core/agent";
import {parseManuscriptPath, type Manuscript} from "@research-assistant/writing";
import {AgentEvidenceReferences} from "./AgentEvidenceReferences";
import type {SourceObject} from "./sourceDirectory";
import {panelItem} from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";

const ports = vi.hoisted(() => ({directory: vi.fn()}));
vi.mock("./sourceDirectory", () => ({sourceDirectory: ports.directory}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: () => ({})}));
vi.mock("../notes/gateways", () => ({getNotesGateway: () => ({})}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({getWritingGateway: () => ({})}));
const project: Manuscript = {id:"project", title:"项目", rootPath:parseManuscriptPath("main.tex"), status:"active", createdAt:"now", updatedAt:"now"};
function file(path:string, kind:"tex"|"other"|"image"): SourceObject {
  return {kind:"manuscriptFile", manuscript:project, file:{manuscriptId:project.id, relativePath:parseManuscriptPath(path), kind, sizeBytes:10, sha256:"hash", modifiedAt:"now"}};
}
it("uses source types for PDF versus raw text, numbers commented text files, and never guesses unavailable sources from titles", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const paper = panelItem();
  const objects = new Map<string, SourceObject>([["pdf",{kind:"pdf", item:paper, attachment:paper.attachments[0]!}], ["md",file("notes.md","other")], ["tex",file("main.tex","tex")], ["image",file("figure.pdf","image")]]);
  ports.directory.mockReturnValue(withSourceNamespaces({value:{kind:"root"}, child:async (id:string) => objects.has(id) ? {value:objects.get(id), child:async () => undefined} : undefined}));
  const evidence: AgentEvidenceInput[] = [
    {type:"annotation",source:"source://library/pdf:#line{1:1-1:1-5}",title:"同一名称",text:"PDF 原文",comment:""},
    {type:"annotation",source:"source://writings/md:#line{1-1:1-5}",title:"同一名称",text:"完整 Markdown 原文",comment:"我的评论"},
    {type:"annotation",source:"source://library/pdf:#line{1:2-1:2-5}",title:"同一名称",text:"PDF 引文",comment:"PDF 评论"},
    {type:"annotation",source:"source://writings/tex:#line{1-1:1-5}",title:"同一名称",text:"代码原文",comment:""},
    {type:"annotation",source:"source://writings/image",title:"同一名称",text:"图像对象",comment:""},
    {type:"annotation",source:"source://notes/missing",title:"fake.md",text:"不可用原文",comment:""},
  ];
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<AgentEvidenceReferences evidence={evidence}/>));
    const cards = [...document.querySelectorAll(".agent-evidence-card")];
    expect(cards[0]?.querySelector("blockquote")).toBeNull(); expect(cards[0]?.querySelector("strong")?.textContent).toBe("PDF 原文");
    expect(cards[1]?.querySelector("blockquote")?.textContent).toBe("完整 Markdown 原文");
    expect(cards[1]?.querySelector(".agent-evidence-user-comment-label")?.textContent).toBe("User comment:");
    expect(cards[1]?.querySelector(".agent-evidence-comment")?.textContent).toBe("我的评论");
    expect(cards[2]?.querySelector(".agent-evidence-user-comment-label")).toBeNull(); expect(cards[2]?.querySelector(".agent-evidence-comment")?.textContent).toBe("PDF 评论");
    expect([...document.querySelectorAll(".agent-evidence-number")].map(item => item.textContent)).toEqual(["1.","2."]);
    expect(cards[4]?.querySelector("blockquote")).toBeNull(); expect(cards[5]?.querySelector("blockquote")).toBeNull();
  } finally {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();}
});
