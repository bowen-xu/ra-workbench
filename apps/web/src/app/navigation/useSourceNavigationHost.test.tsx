// @vitest-environment happy-dom
import {pluginManifestFixtures as builtinPlugins} from "../pluginDeclarations.testSupport";
import {assembleWorkbench} from "../workbenchComposition";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId } from "@research-assistant/core/notes";
import { parseManuscriptPath } from "@research-assistant/writing";
import { previewLiteratureFixtures } from "../runtime/mock/previewLiteratureFixtures";
import { useSourceNavigationHost } from "./useSourceNavigationHost";
import { navigateSource } from "../../core/features/agent/sourceNavigation";
import type { SourceObject } from "../../core/features/agent/sourceDirectory";
const ports = vi.hoisted(() => ({ directory: vi.fn(), load: vi.fn(), paper: vi.fn() }));
vi.mock("../../core/features/agent/sourceDirectory", () => ({sourceDirectory: ports.directory}));
vi.mock("@research-assistant/literature-library-plugin/literatureGateway", () => ({getLiteratureGateway: () => ({})}));
vi.mock("@research-assistant/writing-projects-plugin/writingGateway", () => ({getWritingGateway: () => ({})}));
vi.mock("../../core/features/notes/gateways/index", () => ({getNotesGateway: () => ({loadDocument: ports.load})}));
vi.mock("../../core/services/sourceDomainGateways", () => ({getLiteratureGateway: () => ({get: ports.paper}), getWritingGateway: () => ({})}));
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(cleanup => cleanup()); vi.unstubAllGlobals(); vi.clearAllMocks();});
async function setup(object: SourceObject, available = builtinPlugins.map(plugin => plugin.id),
  extra: Pick<Parameters<typeof useSourceNavigationHost>[0], "domainProvider" | "reading"> = {}) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const node = {value: object, child: async () => node}; ports.directory.mockReturnValue(node);
  const host = document.createElement("div"), root = createRoot(host);
  cleanups.push(() => act(() => root.unmount()));
  const options = {composition: assembleWorkbench([]), domainProvider: (domain: "literature_library" | "writing_projects") => builtinPlugins.find(plugin => plugin.id === domain && available.includes(plugin.id)), chooseView: vi.fn<Parameters<typeof useSourceNavigationHost>[0]["chooseView"]>((_view, _destination, ready) => { ready?.(); return true; }), research: vi.fn(), pdf: vi.fn(), note: vi.fn(), ...extra};
  let navigation!: ReturnType<typeof useSourceNavigationHost>;
  function Harness() {navigation = useSourceNavigationHost(options); return navigation.resourceDialog;}
  await act(async () => root.render(<Harness />));
  return {options, read: () => navigation, rerender: () => act(async () => root.render(<Harness />)), host};
}
it("rejects an unselected domain before querying data or changing navigation, preserving its source identity", async () => {
  const {options} = await setup({kind: "root"}, []);
  localStorage.setItem("stored-reference", "source://writings/project/main.tex");
  await act(async () => expect(navigateSource("source://writings/project/main.tex")).rejects.toThrow("plugin_unavailable"));
  expect(ports.directory).not.toHaveBeenCalled(); expect(options.chooseView).not.toHaveBeenCalled();
  expect(localStorage.getItem("stored-reference")).toBe("source://writings/project/main.tex");
});
it("requires physical PDF pages and routes them with exact line coordinates", async () => {
  const item = previewLiteratureFixtures[0]!.item;
  const {options} = await setup({kind: "pdf", item, attachment: item.attachments[0]!});
  await act(async () => {await expect(navigateSource("source://library/pdf:#line{2-1:2-3}")).rejects.toThrow("缺少实际页码");});
  expect(options.chooseView).not.toHaveBeenCalled();
  await act(async () => navigateSource("source://library/pdf:#line{3:2-1:2-3}"));
  expect(options.pdf).toHaveBeenCalledWith(expect.objectContaining({pageIndex: 2, navigationLineRange: {startLine: 2, startColumn: 1, endLine: 2, endColumn: 3}}));
});
it("validates a note block then passes its block-relative range to the note editor", async () => {
  const snapshot = {page: {id: notePageId("note"), title: "Note", position: 0, status: "active" as const, isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now"}, document: {schemaVersion: 1 as const, pageId: notePageId("note"), blocks: [{id: noteBlockId("b"), type: "paragraph" as const, props: {}, children: [], content: {kind: "inline" as const, items: []}}]}};
  ports.load.mockResolvedValue(snapshot);
  const {options, read} = await setup({kind: "note", snapshot});
  await act(async () => navigateSource("source://notes/note:#block{b}#line{1-1:1-2}"));
  expect(options.note).toHaveBeenCalledWith("note");
  expect(read().note).toMatchObject({blockId: "b", range: {startLine: 1, startColumn: 1, endLine: 1, endColumn: 2}});
  await act(async () => {await expect(navigateSource("source://notes/note:#block{missing}")).rejects.toThrow("不存在");});
  expect(options.note).toHaveBeenCalledOnce();
});
it("waits for writing editor acknowledgment and rejects a superseded request", async () => {
  const manuscript = {id: "project", title: "Project", rootPath: parseManuscriptPath("main.tex"), status: "active" as const, createdAt: "now", updatedAt: "now"};
  const file = {manuscriptId: "project", relativePath: parseManuscriptPath("main.tex"), kind: "tex" as const, sizeBytes: 1, sha256: "hash", modifiedAt: "now"};
  const {read} = await setup({kind: "manuscriptFile", manuscript, file});
  let first!: ReturnType<typeof navigateSource>;
  await act(async () => {first = navigateSource("source://writings/project/main.tex:#line{1-1:1-2}");});
  const failure = expect(first).rejects.toThrow("取代");
  let second!: ReturnType<typeof navigateSource>;
  await act(async () => {second = navigateSource("source://writings/project/main.tex:#line{1-1:1-3}");});
  await failure;
  expect(read().writing?.locator).toMatchObject({range: {endColumn: 3}});
  read().writing!.resolve(); await second;
});

it("accepts block links for writing note files and waits for the shared note surface", async () => {
  const manuscript = {id: "project", title: "Project", rootPath: parseManuscriptPath("main.tex"), status: "active" as const, createdAt: "now", updatedAt: "now"};
  const file = {manuscriptId: "project", relativePath: parseManuscriptPath("notes/idea.ranote"), kind: "other" as const, sizeBytes: 1, sha256: "hash", modifiedAt: "now"};
  const {read} = await setup({kind: "manuscriptFile", manuscript, file});
  let result!: ReturnType<typeof navigateSource>;
  await act(async () => {result = navigateSource("source://writings/project/notes/idea.ranote:#block{b}");});
  expect(read().writing).toMatchObject({path: file.relativePath, locator: {kind: "block", blockId: "b"}});
  const location = {getAnchorRect: () => new DOMRect(1, 2, 3, 4)};
  read().writing!.resolve(location); expect(await result).toBe(location);
  await act(async () => {await expect(navigateSource("source://writings/project/notes/idea.ranote:#line{1-1:1-2}")).rejects.toThrow("block");});
});

it("acknowledges PDF annotation editing only when the reader reports its visible location", async () => {
  const item = previewLiteratureFixtures[0]!.item;
  const {options} = await setup({kind:"pdf",item,attachment:item.attachments[0]!});
  let result!: ReturnType<typeof navigateSource>;
  let finished=false;
  await act(async()=>{result=navigateSource("source://library/pdf:#line{3:2-1:2-3}",{locate:true});void result.then(()=>{finished=true;});});
  expect(finished).toBe(false);
  const source=options.pdf.mock.calls[0]![0];
  const location={getAnchorRect:()=>new DOMRect(1,2,3,4)};
  source.navigationOnLocated(location);
  expect(await result).toBe(location);
});

it("restores physical page and geometry for image annotations and rejects reader failures", async () => {
  const item=previewLiteratureFixtures[0]!.item;
  const {options}=await setup({kind:"pdf",item,attachment:item.attachments[0]!});
  let result!: ReturnType<typeof navigateSource>;
  await act(async()=>{result=navigateSource("source://library/pdf",{locate:true,annotation:{type:"annotation",source:"source://library/pdf",title:"图",text:JSON.stringify({pageNumber:5,rect:{x:0.1,y:0.2,width:0.3,height:0.2}}),comment:""}});});
  const source=options.pdf.mock.calls[0]![0];
  expect(source).toMatchObject({pageIndex:4,rects:[{x:0.1,y:0.2,width:0.3,height:0.2}]});
  const failure=expect(result).rejects.toThrow("位置不可用");
  source.navigationOnError(new Error("位置不可用"));await failure;
});

it.each(["source://agents/prompts/structured-task.md"])("resolves builtin %s before opening its text tab, without a catalog panel",async address=>{
  const {systemDocumentGateway}=await import("../../core/services/systemDocumentGateway");
  const {createMockSystemDocuments}=await import("../runtime/mock/mockSystemDocuments");
  const {OPEN_TEXT_DOCUMENT}=await import("../../core/components/text-editor/textDocumentEvents");
  const gateway=createMockSystemDocuments(localStorage).gateway, read=vi.spyOn(systemDocumentGateway,"read").mockImplementation(gateway.read);
  let opened: import("../../core/components/text-editor/textDocumentEvents").TextDocumentRequest | undefined;
  const listener=(event:Event)=>{event.preventDefault();opened=(event as CustomEvent<typeof opened>).detail;};
  window.addEventListener(OPEN_TEXT_DOCUMENT,listener);
  await setup({kind:"systemDocument",document:await gateway.read("structured-task")});
  try{
    await act(async()=>navigateSource(address));
    expect(read).not.toHaveBeenCalled();expect(ports.directory).toHaveBeenCalled();
    expect(opened?.path).toBe("source://agents/prompts/structured-task.md");
    expect(document.querySelector('[role="dialog"],dialog')).toBeNull();
  }finally{window.removeEventListener(OPEN_TEXT_DOCUMENT,listener);read.mockRestore();}
});

it("unknown asset paths fail before requesting a document or any UI",async()=>{
  const {systemDocumentGateway}=await import("../../core/services/systemDocumentGateway");
  const read=vi.spyOn(systemDocumentGateway,"read");
  await setup({kind:"root"});
  ports.directory.mockImplementation(() => ({value:{kind:"root"},child:async()=>undefined}));
  try{
    await act(async()=>{await expect(navigateSource("source://agents/skills/missing/SKILL.md")).rejects.toThrow("找不到文件对象");});
    expect(read).not.toHaveBeenCalled();expect(ports.directory).toHaveBeenCalled();
  }finally{read.mockRestore();}
});

it("waits for the shell save guard before delivering a standalone writing-note request", async () => {
  const manuscript = {id: "project", title: "Project", rootPath: parseManuscriptPath("main.tex"), status: "active" as const, createdAt: "now", updatedAt: "now"};
  const file = {manuscriptId: "project", relativePath: parseManuscriptPath("notes/note.ranote"), kind: "other" as const, sizeBytes: 1, sha256: "hash", modifiedAt: "now"};
  const {options, read} = await setup({kind: "manuscriptFile", manuscript, file});
  let cancelled: (() => void) | undefined, accepted: (() => void) | undefined;
  options.chooseView.mockImplementation((_owner, _destination, ready, _presentation, cancel) => { accepted = ready; cancelled = cancel; return true; });
  let pending!: ReturnType<typeof navigateSource>;
  await act(async () => { pending = navigateSource("source://writings/project/notes/note.ranote", {presentation: "document"}); });
  expect(options.chooseView).toHaveBeenCalledWith("writing", undefined, expect.any(Function), "document", expect.any(Function));
  expect(read().writing).toBeUndefined(); expect(options.research).not.toHaveBeenCalled();
  const failure = expect(pending).rejects.toThrow("无法打开来源");
  await act(async () => { cancelled?.(); await failure; });
  expect(read().writing).toBeUndefined();
  await act(async () => { pending = navigateSource("source://writings/project/notes/note.ranote", {presentation: "document"}); });
  await act(async () => { accepted?.(); });
  expect(read().writing).toMatchObject({manuscriptId: "project", path: "notes/note.ranote"});
  read().writing!.resolve(); await pending;
});

it("uses an activated owner with no static plugin and does not replay a request made before activation", async () => {
  let owner: object | undefined;
  const {options, rerender} = await setup({kind: "root"}, [], {domainProvider: () => owner});
  await act(async () => {await expect(navigateSource("source://library")).rejects.toThrow("plugin_unavailable");});
  expect(ports.directory).not.toHaveBeenCalled();
  owner = {};
  await rerender();
  expect(options.chooseView).not.toHaveBeenCalled();
  await act(async () => navigateSource("source://library"));
  expect(options.chooseView).toHaveBeenCalledExactlyOnceWith("library", undefined, expect.any(Function), undefined, expect.any(Function));
});

it.each(["failed", "deactivating", "disposed"])("rejects a pending PDF acknowledgment when its owner is %s and isolates its late ack", async () => {
  const item = previewLiteratureFixtures[0]!.item;
  let owner: object | undefined = {};
  const {options, rerender} = await setup({kind: "pdf", item, attachment: item.attachments[0]!}, [], {domainProvider: () => owner});
  let first!: ReturnType<typeof navigateSource>;
  await act(async () => {first = navigateSource("source://library/pdf", {locate: true});});
  const old = options.pdf.mock.calls[0]![0];
  const failure = expect(first).rejects.toThrow("plugin_unavailable");
  owner = undefined;
  await rerender(); await failure;
  owner = {};
  await rerender();
  let second!: ReturnType<typeof navigateSource>, completed = false;
  await act(async () => {
    second = navigateSource("source://library/pdf", {locate: true});
    void second.then(() => {completed = true;});
  });
  old.navigationOnLocated(); old.navigationOnError(new Error("late previous reader"));
  await Promise.resolve();
  expect(completed).toBe(false);
  options.pdf.mock.calls[1]![0].navigationOnLocated();
  await second; expect(completed).toBe(true);
});

it("rejects owner withdrawal during the data read without waiting for it or navigating on its late result", async () => {
  let owner: object | undefined = {};
  const {options, rerender} = await setup({kind: "root"}, [], {domainProvider: () => owner});
  let deliver!: () => void;
  const delayed = new Promise<void>(resolve => {deliver = resolve;});
  const node = {value: {kind: "root" as const}, child: async () => {await delayed; return node;}};
  ports.directory.mockReturnValue(node);
  let result!: ReturnType<typeof navigateSource>;
  await act(async () => {result = navigateSource("source://library/item");});
  const failure = expect(result).rejects.toThrow("plugin_unavailable");
  owner = undefined;
  await rerender(); await failure;
  await act(async () => {deliver();});
  expect(options.chooseView).not.toHaveBeenCalled();
});

it("uses the owner's template slot and closes withdrawn dialog state without reopening on reactivation", async () => {
  let owner: object | undefined = {};
  const Template = ({initialResourceId, onClose}: {initialResourceId?: string; onClose: () => void}) =>
    <button onClick={onClose}>{initialResourceId ?? "Templates"}</button>;
  const {options, rerender, host} = await setup({kind: "structuredCatalog"}, [],
    {domainProvider: () => owner, reading: {ReadingTemplateDialog: Template}});
  await act(async () => navigateSource("source://agents/templates"));
  expect(host.textContent).toBe("Templates");
  await act(async () => host.querySelector("button")!.click());
  expect(host.textContent).toBe("");
  await act(async () => navigateSource("source://agents/templates"));
  owner = undefined; delete options.reading;
  await rerender();
  expect(host.textContent).toBe("");
  owner = {}; options.reading = {ReadingTemplateDialog: Template};
  await rerender();
  expect(host.textContent).toBe("");
  await act(async () => navigateSource("source://agents/templates"));
  expect(host.textContent).toBe("Templates");
});

it("requires the actual literature owner for a structured portrait and calls its contributed detail handler", async () => {
  const requestPortraitDetail = vi.fn(), item = previewLiteratureFixtures[0]!.item;
  ports.paper.mockResolvedValue(item);
  let owner: object | undefined;
  const {options, rerender} = await setup({kind: "structuredObject", resource: {
    id: "portrait", kind: "portrait", paperId: item.id, sourceUri: "source://library/portrait", title: "Portrait",
    revision: 1, content: {}, reviewBaseline: {}, pendingFieldIds: [], reviewedFieldIds: [],
  }}, [], {domainProvider: () => owner, reading: {requestPortraitDetail}});
  await act(async () => {await expect(navigateSource("source://agents/portrait")).rejects.toThrow("plugin_unavailable");});
  expect(options.chooseView).not.toHaveBeenCalled(); expect(ports.paper).not.toHaveBeenCalled();
  owner = {}; await rerender();
  await act(async () => navigateSource("source://agents/portrait"));
  expect(options.research).toHaveBeenCalledExactlyOnceWith({kind: "paper", id: item.id, labelSnapshot: item.title});
  expect(requestPortraitDetail).toHaveBeenCalledExactlyOnceWith(item.id);
});
