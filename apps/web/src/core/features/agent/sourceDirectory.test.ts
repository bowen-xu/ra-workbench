import { expect, it, vi } from "vitest";
import { notePageId, noteBlockId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import { parseManuscriptPath, type Manuscript, type ManuscriptFile } from "@research-assistant/writing";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import { parseSourceLink, resolveSourcePath } from "@research-assistant/core/source";
import { previewLiteratureFixtures } from "../../../app/runtime/mock/previewLiteratureFixtures";
import { sourceDirectory, type SourceDirectoryGateways } from "./sourceDirectory";
import addresses from "../../../../../../tests/fixtures/source-addresses.json";
const item = {...previewLiteratureFixtures[0]!.item, id: "paper"};
const pdf = item.attachments[0]!;
const page: NoteDocumentSnapshot = {page: {id: notePageId("note"), title: "Note", position: 0, status: "active", isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now"}, document: {schemaVersion: 1, pageId: notePageId("note"), blocks: []}};
const manuscript: Manuscript = {id: "project", title: "Project", rootPath: parseManuscriptPath("main.tex"), status: "active", createdAt: "now", updatedAt: "now"};
const file: ManuscriptFile = {manuscriptId: "project", relativePath: parseManuscriptPath("sections/引言.tex"), kind: "tex", sizeBytes: 5, sha256: "hash", modifiedAt: "now"};
const highlight: ReadingNoteBlock = {id: noteBlockId("highlight"), pageId: notePageId("note"), noteType: "highlight", body: "", color: "green", status: "active", isFavorite: false, revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now", source: {paperId: item.id, paperTitle: item.title, attachmentId: pdf.id, contentSha256: pdf.contentSha256, pageIndex: 1, rects: [], quoteExact: "quote", quotePrefix: "", quoteSuffix: "", state: "page_only"}};
function gateways(): SourceDirectoryGateways {
  return {literature: {get: vi.fn(async id => {if (id !== item.id) throw new Error("missing"); return item;})}, notes: {
    loadDocument: vi.fn(async id => {if (id !== page.page.id) throw new Error("missing"); return page;}),
    getReadingNote: vi.fn(async id => { if (id !== highlight.id) throw new Error("missing"); return highlight; }),
    listPages: vi.fn(async () => []), listLiteratureNotes: vi.fn(async id => id === "paper" ? [{page: page.page, preview: ""}] : []), listReadingNotes: vi.fn(async () => [highlight]),
  }, writing: {listManuscripts: vi.fn(async () => [manuscript]), listFiles: vi.fn(async () => [file])}};
}
it("keeps notes independent and attachments as leaf files", async () => {
  const root = sourceDirectory(gateways());
  expect((await resolveSourcePath(root, parseSourceLink("source://notes/note"))).value.kind).toBe("note");
  expect((await resolveSourcePath(root, parseSourceLink(`source://library/paper/${pdf.id}`))).value.kind).toBe("pdf");
  await expect(resolveSourcePath(root, parseSourceLink(`source://library/paper/${pdf.id}/highlight`))).rejects.toThrow();
  await expect(resolveSourcePath(root, parseSourceLink("source://library/paper/note"))).rejects.toThrow();
});
it("uses the writing catalog for nested folders and opaque file identities", async () => {
  const root = sourceDirectory(gateways());
  const nested = await resolveSourcePath(root, parseSourceLink("source://writings/project/sections/%E5%BC%95%E8%A8%80.tex"));
  expect(nested.value).toEqual({kind: "manuscriptFile", manuscript, file});
  expect((await resolveSourcePath(root, parseSourceLink("source://writings/project/sections%2F%E5%BC%95%E8%A8%80.tex"))).value).toEqual(nested.value);
  await expect(resolveSourcePath(root, parseSourceLink("source://writings/project/unknown.tex"))).rejects.toThrow();
});
it("dispatches by namespace without querying other catalogs", async () => {
  const ports = gateways();
  await resolveSourcePath(sourceDirectory(ports), parseSourceLink("source://notes/note"));
  expect(ports.literature.get).not.toHaveBeenCalled();
  expect(ports.writing.listManuscripts).not.toHaveBeenCalled();
  await resolveSourcePath(sourceDirectory(ports), parseSourceLink("source://library/paper"));
  expect(ports.notes.loadDocument).toHaveBeenCalledTimes(1);
  expect(ports.writing.listManuscripts).not.toHaveBeenCalled();
});

it("resolves reading-note IDs directly without inventing a resource-type path", async () => {
  const ports = gateways();
  expect((await resolveSourcePath(sourceDirectory(ports), parseSourceLink("source://notes/highlight"))).value).toEqual({ kind: "note", snapshot: page });
  const annotation = { ...highlight, noteType: "annotation" as const };
  ports.notes.getReadingNote = vi.fn(async () => annotation);
  expect((await resolveSourcePath(sourceDirectory(ports), parseSourceLink("source://notes/reading"))).value).toEqual({ kind: "note", snapshot: page });
});

it.each(["portrait", "readingTemplate"] as const)("resolves a frozen %s tag by exact ID without guessing the active paper",async kind=>{
  const resource:import("@research-assistant/core/structured-resources").StructuredResource={id:`paper-${kind}`,paperId:"paper",kind,title:"相同显示名称",revision:0,sourceUri:`source://library/paper/paper-${kind}`,content:null,reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[]};
  const get=vi.fn(async(id:string)=>{if(id!==resource.id)throw new Error("missing");return resource;});
  const root=sourceDirectory({...gateways(),structured:{get,list:async()=>[resource]}});
  expect(() => parseSourceLink(`source://${resource.id}`)).toThrow();
  expect((await resolveSourcePath(root,parseSourceLink(`source://library/paper/${resource.id}`))).value).toEqual({kind:"structuredObject",resource});
  await expect(resolveSourcePath(root,parseSourceLink("source://library/missing-object"))).rejects.toThrow();
  await expect(resolveSourcePath(root,parseSourceLink(`source://writings/project/${resource.id}`))).rejects.toThrow();
  expect(get).toHaveBeenCalledWith(resource.id);
});

it("resolves registered skill documents and reusable reading assets through the same directory", async () => {
  const document = {id:"structured-task", name:"任务", workflow:"shared" as const, path:"source://agents/prompts/structured-task.md", content:"current", baseline:"before", revision:"1"};
  const template = {id:"template", kind:"readingTemplate" as const, title:"Template", sourceUri:"source://agents/skills/edit-structured-resources/assets/reading/template.json", revision:1, content:null, reviewBaseline:null, pendingFieldIds:[], reviewedFieldIds:[]};
  const read = vi.fn(async () => document), get = vi.fn(async () => template);
  const ports = {...gateways(), documents:{read}, structured:{get,list:async()=>[template]}};
  const root = sourceDirectory(ports);
  expect((await resolveSourcePath(root, parseSourceLink(document.path))).value).toEqual({kind:"systemDocument",document});
  expect(read).toHaveBeenCalledExactlyOnceWith(document.id);
  expect((await resolveSourcePath(root, parseSourceLink(template.sourceUri))).value).toEqual({kind:"structuredObject",resource:template});
  expect(get).toHaveBeenCalledExactlyOnceWith(template.id);
  expect(ports.literature.get).not.toHaveBeenCalled();
  expect(ports.notes.loadDocument).not.toHaveBeenCalled();
  expect(ports.writing.listManuscripts).not.toHaveBeenCalled();
  await expect(resolveSourcePath(root,parseSourceLink("source://agents/prompts/unknown.md"))).rejects.toThrow("找不到文件对象");
  expect(read).toHaveBeenCalledTimes(1);
});
it("loads one writing inventory per resolution and never probes structured resources for an attachment", async () => {
  const ports = gateways(), get = vi.fn(async () => {throw new Error("unrelated query");});
  const root = sourceDirectory({...ports, structured:{get,list:async()=>[]}});
  await resolveSourcePath(root, parseSourceLink("source://writings/project/sections/%E5%BC%95%E8%A8%80.tex"));
  expect(ports.writing.listFiles).toHaveBeenCalledTimes(1);
  await resolveSourcePath(root, parseSourceLink(`source://library/paper/${pdf.id}`));
  expect(get).not.toHaveBeenCalled();
});
it("rejects a document gateway response for a different registered identity", async () => {
  const read = vi.fn(async () => ({id:"reading-reference",name:"Other document",workflow:"reading" as const,path:"source://agents/skills/edit-structured-resources/references/reading.md",content:"text",baseline:"text",revision:"1"}));
  await expect(resolveSourcePath(sourceDirectory({...gateways(),documents:{read}}),parseSourceLink("source://agents/prompts/structured-task.md"))).rejects.toThrow("找不到文件对象");
});
it.each(addresses.unregistered)("rejects unregistered %s without querying any resource gateway", async path => {
  const ports = {...gateways(), documents:{read:vi.fn()}, structured:{get:vi.fn(),list:vi.fn()}};
  await expect(resolveSourcePath(sourceDirectory(ports),parseSourceLink(path))).rejects.toThrow("找不到文件对象");
  expect(ports.documents.read).not.toHaveBeenCalled();
  expect(ports.structured.get).not.toHaveBeenCalled();
  expect(ports.structured.list).not.toHaveBeenCalled();
  expect(ports.literature.get).not.toHaveBeenCalled();
  expect(ports.notes.loadDocument).not.toHaveBeenCalled();
  expect(ports.notes.getReadingNote).not.toHaveBeenCalled();
  expect(ports.writing.listManuscripts).not.toHaveBeenCalled();
});
it("reserves separate Agent, custom and general-asset directories",async()=>{
  const root=sourceDirectory(gateways());
  for(const path of ["source://agents/", "source://agents/tools/", "source://agents/plugins/", "source://agents/custom/", "source://agents/custom/skills/", "source://agents/custom/tools/", "source://assets/"]){
    expect((await resolveSourcePath(root,parseSourceLink(path))).value.kind).toBe("directory");
  }
});
