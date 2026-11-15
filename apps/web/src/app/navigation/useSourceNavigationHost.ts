import { getStructuredResourceGateway } from "../../core/services/structuredResourceGateway";
import type {WorkbenchComposition} from "../workbenchComposition";
import { createElement, useEffect, useRef, useState } from "react";
import { resolveSourcePath } from "@research-assistant/core/source";
import { notePageId, type NoteLinkTarget, type NotePageId } from "@research-assistant/core/notes";
import { getLiteratureGateway } from "../../core/services/sourceDomainGateways";
import { getNotesGateway } from "../../core/features/notes/gateways/index";
import { getWritingGateway } from "../../core/services/sourceDomainGateways";
import type { ReaderSourceNavigation } from "../../core/contracts/sourceNavigation";
import { sourceDirectory } from "../../core/features/agent/sourceDirectory";
import { SOURCE_NAVIGATION_EVENT, type SourceNavigationRequest, type SourceNavigationLocation } from "../../core/features/agent/sourceNavigation";
import {systemDocumentGateway} from "../../core/services/systemDocumentGateway";
import {openResolvedSystemDocument} from "../../core/features/system-documents/openSystemDocument";

import {isWritingNotePath} from "@research-assistant/writing";
import type {PluginReadingContributions} from "@research-assistant/workbench-sdk/activation";

import type { WritingSourceNavigation, NoteSourceNavigation } from "../../core/contracts/sourceNavigation";
export type { WritingSourceNavigation, NoteSourceNavigation } from "../../core/contracts/sourceNavigation";
interface Options {
  readonly composition: WorkbenchComposition;
  /** Actual selected/activated owner identity, not an arbitrary route string. */
  readonly domainProvider?: (domain: "literature_library" | "writing_projects") => object | undefined;
  readonly reading?: PluginReadingContributions;
  readonly chooseView: (view: "library" | "writing" | "notes", destination?: string, ready?: () => void, presentation?: "workspace" | "document", cancelled?: () => void) => boolean;
  readonly research: (target: NoteLinkTarget) => void;
  readonly pdf: (source: ReaderSourceNavigation) => void;
  readonly note: (id: NotePageId) => void;
}
export function useSourceNavigationHost(options: Options) {
  const current = useRef(options); current.current = options;
  const invalidateOwner = useRef<() => void>(() => undefined);
  const [template,setTemplate]=useState<{readonly resourceId: string;
    readonly component: NonNullable<PluginReadingContributions["ReadingTemplateDialog"]>} | null>(null);
  const [writing, setWriting] = useState<WritingSourceNavigation>();
  const [note, setNote] = useState<NoteSourceNavigation>();
  useEffect(() => {
    let sequence = 0, active = true;
    let pending: SourceNavigationRequest | undefined;
    const listener = (event: Event) => {
      const incoming = (event as CustomEvent<SourceNavigationRequest>).detail;
      event.preventDefault();
      pending?.reject(new Error("来源导航已被新请求取代"));
      const request: SourceNavigationRequest = {...incoming,
        resolve: location => {if (pending === request) pending = undefined; incoming.resolve(location);},
        reject: reason => {if (pending === request) pending = undefined; incoming.reject(reason);}};
      pending = request;
      setWriting(undefined);
      const token = ++sequence;
      let owner: {readonly domain: "literature_library" | "writing_projects"; readonly provider: object} | undefined;
      const provider = (domain: "literature_library" | "writing_projects") => current.current.domainProvider
        ? current.current.domainProvider(domain) : undefined;
      const unavailable = (domain: string) => new Error(`plugin_unavailable：${domain} 未装配；来源与原资源已保留。`);
      const assertCurrent = () => {
        if (!active || token !== sequence) throw new Error("来源导航已被新请求取代");
        if (owner && provider(owner.domain) !== owner.provider) throw unavailable(owner.domain);
      };
      const requireOwner = (domain: "literature_library" | "writing_projects") => {
        const selected = provider(domain);
        if (!selected) throw unavailable(domain);
        owner = {domain, provider: selected};
        assertCurrent();
      };
      // This is the original single pending request, not a replay queue. Owner
      // withdrawal must finish it even if its editor never acknowledges it.
      invalidateOwner.current = () => {
        if (pending !== request || !owner || provider(owner.domain) === owner.provider) return;
        sequence++;
        request.reject(unavailable(owner.domain));
        setWriting(value => value?.token === token ? undefined : value);
        setNote(value => value?.token === token ? undefined : value);
      };
      void (async () => {
        const domain = request.link.path[0] === "library" ? "literature_library" : request.link.path[0] === "writings" ? "writing_projects" : undefined;
        if (domain) requireOwner(domain);
        const notes = getNotesGateway();
        const root = sourceDirectory({ literature: getLiteratureGateway(), notes, writing: getWritingGateway(), structured:getStructuredResourceGateway(), documents:systemDocumentGateway });
        const object = (await resolveSourcePath(root, request.link)).value;
        assertCurrent();
        const locator = request.link.locator;
        async function visitWorkspace(view: "library" | "writing" | "notes", destination?: string): Promise<void> {
          await new Promise<void>((resolve, reject) => {
            const cancelled = () => reject(new Error("当前编辑状态尚未保存，无法打开来源"));
            if (!current.current.chooseView(view, destination, resolve, request.options?.presentation, cancelled)) cancelled();
          });
          assertCurrent();
        }
        if (object.kind === "systemDocument") {
          if (locator) throw new Error("内置文档不支持此定位符");
          openResolvedSystemDocument(object.document, systemDocumentGateway, () => active && token === sequence);
          if (active && token === sequence) request.resolve();
          return;
        }
        if (object.kind === "directory" || object.kind === "root") {
          const view = request.link.path[0] === "library" ? "library" : request.link.path[0] === "notes" ? "notes" : request.link.path[0] === "writings" ? "writing" : undefined;
          if (view) await visitWorkspace(view);
          request.resolve(); return;
        }
        const located = (location?: SourceNavigationLocation) => {
          try {assertCurrent(); request.resolve(location);} catch (reason) {request.reject(reason);}
        };
        async function openNote(id: string) {
          if (locator && locator.kind !== "block") throw new Error("笔记定位需要 block 定位符");
          if (locator?.kind === "block") {
            const snapshot = await notes.loadDocument(notePageId(id));
            const contains = (blocks: typeof snapshot.document.blocks): boolean => blocks.some(block => block.id === locator.blockId || contains(block.children));
            if (!contains(snapshot.document.blocks)) throw new Error("来源笔记块不存在或已被删除");
          }
          assertCurrent();
          await visitWorkspace("notes", id);
          setNote(locator?.kind === "block" ? { pageId: id, blockId: locator.blockId, token, ...(locator.range ? { range: locator.range } : {}), ...(request.options?.locate ? {onLocated: located, onError: request.reject} : {}) } : undefined);
          current.current.note(notePageId(id));
        }
        if(object.kind === "structuredCatalog" || object.kind === "structuredObject" && object.resource.kind === "readingTemplate") {
          if(locator) throw new Error("模板资源不支持此定位符");
          requireOwner("literature_library");
          const reading = current.current.reading ?? {};
          if (!reading.ReadingTemplateDialog) throw new Error("plugin_unavailable：阅读模板界面需要文献管理插件；原资源已保留。");
          setTemplate({resourceId: object.kind === "structuredObject" ? object.resource.id : "",
            component: reading.ReadingTemplateDialog});
        } else if(object.kind === "structuredObject") {
          if(locator) throw new Error("结构化资源不支持此定位符");
          const paperId=object.resource.paperId;if(!paperId)throw new Error("资源缺少所属文献");
          requireOwner("literature_library");
          await visitWorkspace("library");
          const paper=await getLiteratureGateway().get(paperId);
          assertCurrent();
          current.current.research({kind:"paper",id:paperId,labelSnapshot:paper.title});
          if(object.resource.kind === "portrait") current.current.reading?.requestPortraitDetail?.(paperId);
          window.setTimeout(()=>{
            if (active && token === sequence && owner && provider(owner.domain) === owner.provider)
              document.querySelector<HTMLElement>(`[data-structured-resource="${object.resource.id}"]`)?.scrollIntoView({block:"nearest"});
          },0);
        } else if (object.kind === "note") { await openNote(object.snapshot.page.id); if (request.options?.locate && locator?.kind === "block") return; }
        else if (object.kind === "highlight" && locator?.kind === "block") {
          if (!object.note.pageId) throw new Error("高亮没有关联的笔记文档");
          await openNote(object.note.pageId);
          if (request.options?.locate) return;
        } else if (object.kind === "pdf" || object.kind === "highlight") {
          if (locator && locator.kind !== "line") throw new Error("PDF 定位需要带实际页码的 line 定位符");
          if (locator?.kind === "line" && locator.pageNumber === undefined) throw new Error("PDF 行列定位缺少实际页码（从 1 开始）");
          await visitWorkspace("library");
          const source: ReaderSourceNavigation = object.kind === "highlight" ? object.note.source : {
            paperId: object.item.id, paperTitle: object.item.title, attachmentId: object.attachment.id,
            contentSha256: object.attachment.contentSha256, pageIndex: (locator?.kind === "line" ? locator.pageNumber! : 1) - 1,
            rects: [], quoteExact: "", quotePrefix: "", quoteSuffix: "", state: "page_only",
          };
          // Geometry-only PDF annotations retain their page/region in captured metadata.
          let region: {pageIndex: number; rects: typeof source.rects} | undefined;
          if (!locator && request.options?.annotation) {
            try {
              const data = JSON.parse(request.options.annotation.text) as {pageNumber?: number; rect?: Record<string, unknown>};
              const rect = data.rect;
              if (Number.isInteger(data.pageNumber) && data.pageNumber! > 0 && rect && ["x","y","width","height"].every(key => typeof rect[key] === "number" && Number.isFinite(rect[key]) && (rect[key] as number) >= 0 && (rect[key] as number) <= 1)
                && (rect.width as number) > 0 && (rect.height as number) > 0 && (rect.x as number) + (rect.width as number) <= 1 && (rect.y as number) + (rect.height as number) <= 1)
                region = {pageIndex: data.pageNumber! - 1, rects: [rect as unknown as typeof source.rects[number]]};
            } catch { /* Ordinary text annotations have no geometry envelope. */ }
          }
          current.current.pdf({ ...source, ...(region ?? {}), ...(locator?.kind === "line" ? { pageIndex: locator.pageNumber! - 1, navigationLineRange: locator.range } : {}),
            ...(request.options?.locate ? {navigationOnLocated: located, navigationOnError: request.reject} : {}) });
          if (request.options?.locate) return;
        } else {
          const project = object.kind === "manuscript" || object.kind === "folder" || object.kind === "manuscriptFile";
          if (locator && object.kind !== "manuscriptFile") throw new Error("该虚拟目录没有可定位的正文");
          if (object.kind === "manuscriptFile" && locator) {
            if (isWritingNotePath(object.file.relativePath)) {
              if (locator.kind !== "block") throw new Error("笔记定位需要 block 定位符");
            } else if (locator.kind === "block" || locator.pageNumber !== undefined) throw new Error("文本文件需要不带页码的 line 定位符");
          }
          if (object.kind === "manuscriptFile" && locator && object.file.kind === "image") throw new Error("图片文件不支持正文行列定位");
          if (object.kind === "manuscriptFile" && object.file.kind === "image" && request.options?.locate) throw new Error("当前写作图片没有可显示的批注编辑位置");
          await visitWorkspace(project ? "writing" : "library");
          if (object.kind === "paper") current.current.research({ kind: "paper", id: object.item.id, labelSnapshot: object.item.title });
          else if (project) {
            current.current.research({ kind: "manuscript", id: object.manuscript.id, labelSnapshot: object.manuscript.title });
            if (object.kind === "manuscriptFile") {
              setWriting({ manuscriptId: object.manuscript.id, path: object.file.relativePath, ...(locator ? { locator } : {}), ...(request.options?.locate ? {locate:true} : {}), token, resolve: located, reject: request.reject });
              return;
            }
          } else throw new Error("无法打开来源对象");
        }
        request.resolve();
      })().catch(request.reject);
    };
    window.addEventListener(SOURCE_NAVIGATION_EVENT, listener);
    return () => { active = false; sequence++; pending?.reject(new Error("来源导航界面已关闭")); window.removeEventListener(SOURCE_NAVIGATION_EVENT, listener); };
  }, []);
  useEffect(() => {invalidateOwner.current();});
  const ReadingTemplateDialog = (options.reading ?? {}).ReadingTemplateDialog;
  useEffect(() => {if (template && template.component !== ReadingTemplateDialog) setTemplate(null);}, [template, ReadingTemplateDialog]);
  return { writing, note, resourceDialog:!template || template.component !== ReadingTemplateDialog ? null
    : createElement(template.component,{...(template.resourceId ? {initialResourceId:template.resourceId} : {}),onClose:()=>setTemplate(null)}) };
}
