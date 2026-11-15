import { formatBibliography } from "@research-assistant/core/citation";
import type { SourceLocator } from "@research-assistant/core/source";
import type { SourceObject } from "./sourceDirectory";

/** Display names only; routing continues to use the original object identities. */
export function sourceLinkHint(object: SourceObject, locator?: SourceLocator, noteTitle?: string): readonly string[] {
  let lines: string[];
  switch (object.kind) {
    case "paper": lines = [formatBibliography([object.item], "nature").replace(/^\s*1\.\s*/u, "")]; break;
    case "pdf": lines = [formatBibliography([object.item], "nature").replace(/^\s*1\.\s*/u, ""), object.attachment.displayName]; break;
    case "note": {
      const title = object.snapshot.page.title || "未命名笔记";
      return [locator?.kind === "block" ? `block in ${title}` : title];
    }
    case "highlight": {
      if (locator?.kind === "block") return [`block in ${noteTitle || "高亮笔记"}`];
      lines = [object.note.source.paperTitle, "高亮"]; break;
    }
    case "manuscript": lines = [`写作项目：${object.manuscript.title}`]; break;
    case "folder": lines = [`写作项目：${object.manuscript.title}`, object.path]; break;
    case "manuscriptFile": lines = [`写作项目：${object.manuscript.title}`, object.file.relativePath]; break;
    case "structuredCatalog": return ["全局阅读模板库"];
    case "structuredObject": return [object.resource.title,object.resource.revision === 0 ? "待生成" : `v${object.resource.revision}`];
    case "directory": return [object.title];
    case "systemDocument": return [object.document.name];
    case "root": return ["来源"];
  }
  if (locator?.kind === "line") {
    const { range, pageNumber } = locator;
    const location = `${pageNumber === undefined ? "" : `第 ${pageNumber} 页 · `}line ${range.startLine}-${range.startColumn}:${range.endLine}-${range.endColumn}`;
    if (object.kind === "pdf" || object.kind === "manuscriptFile") lines[1] += ` · ${location}`;
    else lines.push(location);
  }
  return lines;
}
