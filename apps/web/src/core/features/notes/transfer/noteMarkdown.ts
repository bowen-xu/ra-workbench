import {referenceSourcePath} from "@research-assistant/core/source";
import { BlockNoteEditor } from "@blocknote/core";
import {
  isAllowedNoteHref,
  noteImageDisplayName,
  type NoteBlockV1,
  type NoteDocumentV1,
  type NoteInlineContentV1,
  type NotePageId,
} from "@research-assistant/core/notes";

import { blockNoteToNoteDocument, noteDocumentToBlockNote } from "../editor/blockNoteAdapter";
import { noteEditorSchema, type NoteEditorBlock } from "../editor/noteEditorSchema";

export const MAX_MARKDOWN_IMPORT_BYTES = 8 * 1024 * 1024;

function assertMarkdownImportSize(markdown: string): void {
  if (new TextEncoder().encode(markdown).byteLength > MAX_MARKDOWN_IMPORT_BYTES) {
    throw new Error("Markdown 文件不能超过 8 MB");
  }
}

function escapeMarkdown(value: string): string {
  return value.replace(/([\\`*_[\]<>])/g, "\\$1");
}

function inlineMarkdown(items: readonly NoteInlineContentV1[]): string {
  return items.map((item) => {
    if (item.type === "link") return `[${inlineMarkdown(item.content)}](${item.href})`;
    if (item.type === "inlineFormula") return `$${item.source}$`;
    if (item.type === "mention") return `@[${escapeMarkdown(item.labelSnapshot)}](${referenceSourcePath(item.targetKind, item.targetId)})`;
    let text = escapeMarkdown(item.text);
    for (const style of item.styles) {
      if (style === "bold") text = `**${text}**`;
      else if (style === "italic") text = `*${text}*`;
      else if (style === "strike") text = `~~${text}~~`;
      else if (style === "code") text = `\`${text}\``;
      else if (style === "underline") text = `<u>${text}</u>`;
    }
    return text;
  }).join("");
}

function hasToggle(block: NoteBlockV1): boolean {
  return block.type === "toggleListItem" || block.type === "heading" && block.props.isToggleable === true || block.children.some(hasToggle);
}

function blockMarkdown(block: NoteBlockV1, depth: number, assetPaths: ReadonlyMap<string, string>): string {
  const content = block.content;
  let line = "";
  if (content.kind === "formula") line = `$$\n${content.source}\n$$`;
  else if (content.kind === "asset") {
    const path = assetPaths.get(content.assetId);
    if (path === undefined) throw new Error("note_asset_export_path_missing：附件缺少已登记的导出路径。");
    // Encode destinations, not filesystem names; parentheses left by
    // encodeURIComponent would still terminate a Markdown link destination.
    const target = ["..", ...path.split("/")].map(part => encodeURIComponent(part)
      .replace(/[!'()*]/gu, value => `%${value.charCodeAt(0).toString(16).toUpperCase()}`)).join("/");
    line = block.type === "image" ? `![${escapeMarkdown(content.caption || "图片")}](${target})` : `[${escapeMarkdown(content.displayName)}](${target})`;
  } else if (content.kind === "reference") {
    line = `> [!research-reference] ${escapeMarkdown(content.labelSnapshot)}\n> ${escapeMarkdown(content.quoteSnapshot)}\n> ${referenceSourcePath(content.targetKind, content.targetId)}`;
  } else if (content.kind === "table") {
    const rows = content.rows.map((row) => row.map((cell) => inlineMarkdown(cell.items)));
    const width = Math.max(1, ...rows.map((row) => row.length));
    const normalized = rows.map((row) => Array.from({ length: width }, (_, index) => row[index] ?? ""));
    line = normalized.length === 0 ? "| |\n| --- |" : [
      `| ${normalized[0]!.join(" | ")} |`,
      `| ${Array.from({ length: width }, () => "---").join(" | ")} |`,
      ...normalized.slice(1).map((row) => `| ${row.join(" | ")} |`),
    ].join("\n");
    if (typeof block.props.caption === "string" && block.props.caption) line += `\n\n${escapeMarkdown(block.props.caption)}`;
  } else if (content.kind === "unsupported") {
    line = `> [!warning] 暂不支持的笔记块：${escapeMarkdown(content.originalType)}\n\n\`\`\`\`json\n${content.rawJson}\n\`\`\`\``;
  } else if (content.kind === "none") line = "---";
  else {
    const text = inlineMarkdown(content.items);
    if (block.type === "heading") line = `${"#".repeat(Number(block.props.level ?? 2))} ${text}`;
    else if (block.type === "bulletListItem") line = `- ${text}`;
    else if (block.type === "numberedListItem") line = `1. ${text}`;
    else if (block.type === "checkListItem") line = `- [${block.props.checked === true ? "x" : " "}] ${text}`;
    else if (block.type === "quote") line = `> ${text}`;
    else if (block.type === "codeBlock") line = `\`\`\`\n${text}\n\`\`\``;
    else line = text;
  }
  if (hasToggle(block)) {
    // CommonMark has no toggle block. Use upstream's native <details>/<summary>
    // interchange so Markdown import restores the type, title styles and children.
    const editor = BlockNoteEditor.create({ schema: noteEditorSchema });
    const shell = noteDocumentToBlockNote({schemaVersion: 1, pageId: "markdown-export" as NotePageId, blocks: [{...block, children: []}]}, editor.schema.blockSchema);
    const childMarkdown = block.children.map(child => blockMarkdown(child, 0, assetPaths)).join("\n\n");
    const children = childMarkdown ? editor.tryParseMarkdownToBlocks(childMarkdown) : [];
    const template = document.createElement("template");
    template.innerHTML = editor.blocksToHTMLLossy(shell.map(item => ({...item, children})));
    const ownToggle = block.type === "toggleListItem" || block.type === "heading" && block.props.isToggleable === true;
    const details = template.content.querySelector("details");
    if (ownToggle && !details) throw new Error("note_toggle_export_failed：可折叠块缺少 details 结构。");
    // Keep list ancestors as native HTML too: a bare <details> directly in a
    // Markdown <li> would make upstream classify the parent as the toggle.
    const html = ownToggle && details ? details.outerHTML : template.innerHTML;
    return html.split("\n").map(part => `${"  ".repeat(depth)}${part}`).join("\n");
  }
  const indent = "  ".repeat(depth);
  const own = line.split("\n").map((part) => `${indent}${part}`).join("\n");
  const children = block.children.map((child) => blockMarkdown(child, depth + 1, assetPaths)).join("\n\n");
  return children === "" ? own : `${own}\n\n${children}`;
}

export function noteDocumentToMarkdown(document: NoteDocumentV1,
  assetPaths: ReadonlyMap<string, string> = noteAssetExportPaths(collectNoteAssets(document)),
): string {
  return `${document.blocks.map((block) => blockMarkdown(block, 0, assetPaths)).join("\n\n").trim()}\n`;
}

export function markdownToNoteDocument(pageId: NotePageId, markdown: string): NoteDocumentV1 {
  assertMarkdownImportSize(markdown);
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema, links: { isValidLink: isAllowedNoteHref } });
  const blocks = editor.tryParseMarkdownToBlocks(markdown) as unknown as NoteEditorBlock[];
  const document = blockNoteToNoteDocument(pageId, blocks);
  const sanitize = (block: NoteBlockV1): NoteBlockV1 => {
    const children = block.children.map(sanitize);
    if (block.content.kind !== "asset" || /^[a-f\d]{64}$/iu.test(block.content.assetId)) return { ...block, children };
    return {
      id: block.id,
      type: "paragraph",
      props: {},
      content: { kind: "inline", items: [{ type: "text", text: `未导入的外部资源：${block.content.displayName}（${block.content.assetId}）`, styles: [] }] },
      children,
    };
  };
  return { ...document, blocks: document.blocks.map(sanitize) };
}

export interface NoteMarkdownImportPreview {
  readonly document: NoteDocumentV1;
  readonly blockCount: number;
  readonly warnings: readonly string[];
}

export function previewMarkdownImport(pageId: NotePageId, markdown: string): NoteMarkdownImportPreview {
  const externalAssetPattern = /!\[[^\]]*\]\(([^)]+)\)|\[[^\]]+\]\((?:file:|\.\.?\/)([^)]+)\)/giu;
  const externalAssets = [...markdown.matchAll(externalAssetPattern)].length;
  const document = markdownToNoteDocument(pageId, markdown);
  let blockCount = 0;
  const visit = (blocks: readonly NoteBlockV1[]): void => {
    for (const block of blocks) {
      blockCount += 1;
      visit(block.children);
    }
  };
  visit(document.blocks);
  return {
    document,
    blockCount,
    warnings: externalAssets === 0 ? [] : [`${externalAssets} 个外部图片或附件未复制，已保留为文本说明`],
  };
}

export function safeExportName(value: string): string {
  const normalized = value.normalize("NFKC").replace(/[\\/:*?"<>|\u0000-\u001f]/g, "-").replace(/\s+/g, " ").trim();
  return (normalized || "untitled").slice(0, 100);
}

/** One chosen filename per content identity, shared by Markdown and file copy. */
export function noteAssetExportPaths(assets: Iterable<{readonly id: string; readonly displayName: string; readonly mimeType: string}>): ReadonlyMap<string, string> {
  return new Map(Array.from(assets, asset => [asset.id, `assets/${noteImageDisplayName(asset.id, asset.mimeType) ?? `${asset.id}-${safeExportName(asset.displayName)}`}`]));
}

export function collectNoteAssets(document: NoteDocumentV1): readonly { readonly id: string; readonly displayName: string; readonly mimeType: string }[] {
  const assets = new Map<string, { readonly id: string; readonly displayName: string; readonly mimeType: string }>();
  const visit = (blocks: readonly NoteBlockV1[]): void => {
    for (const block of blocks) {
      if (block.content.kind === "asset") assets.set(block.content.assetId, { id: block.content.assetId, displayName: block.content.displayName, mimeType: block.content.mimeType });
      visit(block.children);
    }
  };
  visit(document.blocks);
  return [...assets.values()];
}
