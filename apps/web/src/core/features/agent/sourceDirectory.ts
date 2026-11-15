import {systemDocumentIds, systemDocumentPath, type SystemDocument, type SystemDocumentGateway} from "@research-assistant/core/agent";
import { type StructuredResource } from "@research-assistant/core/structured-resources";
import type { StructuredResourceGateway } from "../../services/structuredResourceGateway";
import type { AttachmentSummary, LiteratureItem } from "@research-assistant/core/literature";
import { noteBlockId, notePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import type { NotesGateway, NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { Manuscript, ManuscriptFile } from "@research-assistant/writing";
import type { SourceDirectoryNode } from "@research-assistant/core/source";
import type { LiteratureSourceAccess } from "../../services/sourceDomainGateways";
import type { WritingSourceAccess } from "../../services/sourceDomainGateways";

export type SourceObject = { readonly kind: "root" }
  | { readonly kind: "directory"; readonly title: string }
  | { readonly kind: "systemDocument"; readonly document: SystemDocument }
  | { readonly kind: "structuredCatalog" }
  | { readonly kind: "structuredObject"; readonly resource: StructuredResource }
  | { readonly kind: "paper"; readonly item: LiteratureItem }
  | { readonly kind: "pdf"; readonly item: LiteratureItem; readonly attachment: AttachmentSummary }
  | { readonly kind: "note"; readonly snapshot: NoteDocumentSnapshot }
  | { readonly kind: "highlight"; readonly note: ReadingNoteBlock }
  | { readonly kind: "manuscript"; readonly manuscript: Manuscript }
  | { readonly kind: "folder"; readonly manuscript: Manuscript; readonly path: string }
  | { readonly kind: "manuscriptFile"; readonly manuscript: Manuscript; readonly file: ManuscriptFile };
export interface SourceDirectoryGateways {
  readonly documents?: Pick<SystemDocumentGateway, "read">;
  readonly structured?: Pick<StructuredResourceGateway,"get" | "list">;
  readonly literature: Pick<LiteratureSourceAccess, "get">;
  readonly notes: Pick<NotesGateway, "loadDocument" | "listPages" | "listLiteratureNotes" | "listReadingNotes" | "getReadingNote">;
  readonly writing: Pick<WritingSourceAccess, "listManuscripts" | "listFiles">;
}
type Node = SourceDirectoryNode<SourceObject>;
const leaf = (value: SourceObject): Node => ({ value, child: async () => undefined });

/** Every node resolves only its own children; IDs are never inferred from labels. */
export function sourceDirectory(gateways: SourceDirectoryGateways): Node {
  const note = (snapshot: NoteDocumentSnapshot): Node => leaf({kind: "note", snapshot});
  const writing = (manuscript: Manuscript, path = "", pendingFiles?: Promise<readonly ManuscriptFile[]>): Node => ({
    value: path ? { kind: "folder", manuscript, path } : { kind: "manuscript", manuscript },
    child: async id => {
      const files = await (pendingFiles ??= gateways.writing.listFiles(manuscript.id));
      const target = path ? `${path}/${id}` : id;
      const file = files.find(file => file.relativePath === target);
      if (file) return leaf({ kind: "manuscriptFile", manuscript, file });
      return files.some(file => file.relativePath.startsWith(`${target}/`)) ? writing(manuscript, target, pendingFiles) : undefined;
    },
  });
  const pdf = (item: LiteratureItem, attachment: AttachmentSummary): Node => leaf({kind: "pdf", item, attachment});
  const paper = (item: LiteratureItem): Node => ({ value: { kind: "paper", item }, child: async id => {
    const attachment = item.attachments.find(file => file.id === id);
    if (attachment) return pdf(item, attachment);
    const managed=await gateways.structured?.get(id).catch(()=>undefined);
    if(managed && managed.paperId === item.id) return leaf({kind:"structuredObject",resource:managed});
    return undefined;
  } });
  const agentCategories = ["prompts", "skills", "tools", "plugins"];
  const agents = (parts: readonly string[] = []): Node => ({value: {kind: "directory", title: parts[0] === "custom" ? "用户 Agent 资源" : "Agent 资源"}, child: async id => {
    const path = [...parts, id];
    if (path[0] === "custom") {
      return path.length === 1 || path.length === 2 && agentCategories.includes(id) ? agents(path) : undefined;
    }
    const reading = ["skills", "edit-structured-resources", "assets", "reading"];
    if (path.every((part, index) => reading[index] === part)) {
      if (path.length < reading.length) return agents(path);
      if (!gateways.structured) return undefined;
      return {value: {kind: "structuredCatalog"}, child: async filename => {
        if (!filename.endsWith(".json")) return undefined;
        const resource = await gateways.structured!.get(filename.slice(0, -5)).catch(() => undefined);
        return resource?.kind === "readingTemplate" && !resource.paperId ? leaf({kind: "structuredObject", resource}) : undefined;
      }};
    }
    const registered = systemDocumentIds.map(id => ({id, parts: systemDocumentPath(id)!.slice("source://agents/".length).split("/")}));
    const exact = registered.find(entry => entry.parts.length === path.length && path.every((part, index) => entry.parts[index] === part));
    if (exact && gateways.documents) {
      const document = await gateways.documents.read(exact.id);
      if (document.id !== exact.id || document.path !== systemDocumentPath(exact.id)) throw new Error("找不到文件对象");
      return leaf({kind: "systemDocument", document});
    }
    if (exact) return undefined;
    return path.length === 1 && agentCategories.includes(id) || registered.some(entry => entry.parts.length > path.length && path.every((part, index) => entry.parts[index] === part)) ? agents(path) : undefined;
  }});
  const directory = (title: string, child: Node["child"]): Node => ({value: {kind: "directory", title}, child});
  return {value: {kind: "root"}, child: async root => {
    if (root === "agents") return agents();
    if (root === "assets") return directory("通用资产", async () => undefined);
    if (root === "library") return directory("所有文献", async id => {
      const item = await gateways.literature.get(id).catch(() => undefined);
      return item ? paper(item) : undefined;
    });
    if (root === "notes") return directory("所有笔记", async id => {
      const snapshot = await gateways.notes.loadDocument(notePageId(id)).catch(() => undefined);
      if (snapshot) return note(snapshot);
      const reading = await gateways.notes.getReadingNote(noteBlockId(id)).catch(() => undefined);
      if (reading?.pageId) return note(await gateways.notes.loadDocument(reading.pageId));
      return reading?.noteType === "highlight" ? leaf({kind: "highlight", note: reading}) : undefined;
    });
    if (root === "writings") return directory("所有写作项目", async id => {
      const project = (await gateways.writing.listManuscripts()).find(project => project.id === id);
      return project ? writing(project) : undefined;
    });
    return undefined;
  }};
}
