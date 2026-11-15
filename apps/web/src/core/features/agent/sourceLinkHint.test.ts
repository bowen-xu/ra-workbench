import { expect, it } from "vitest";
import { formatBibliography } from "@research-assistant/core/citation";
import { parseManuscriptPath } from "@research-assistant/writing";
import { createEmptyNoteDocument, notePageId, noteBlockId } from "@research-assistant/core/notes";
import type { SourceLocator } from "@research-assistant/core/source";
import { sourceLinkHint } from "./sourceLinkHint";
import { panelItem } from "../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";

const item = panelItem();
const range = { startLine: 2, startColumn: 3, endLine: 4, endColumn: 5 };
const line: SourceLocator = { kind: "line", pageNumber: 7, range };
const page = { id: notePageId("private-note-id"), title: "实验笔记", position: 0, status: "active" as const,
  isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" };
const snapshot = { page, document: createEmptyNoteDocument(page.id, noteBlockId("private-block-id")) };

it("formats a literature citation with the existing Nature bibliography formatter", () => {
  expect(sourceLinkHint({ kind: "paper", item })).toEqual([formatBibliography([item], "nature").replace(/^\s*1\.\s*/u, "")]);
});
it("separates the PDF citation, filename and physical page/line range", () => {
  const object = { kind: "pdf" as const, item, attachment: item.attachments[0]! };
  expect(sourceLinkHint(object)).toEqual([formatBibliography([item], "nature").replace(/^\s*1\.\s*/u, ""), object.attachment.displayName]);
  expect(sourceLinkHint(object, line)).toEqual([
    formatBibliography([item], "nature").replace(/^\s*1\.\s*/u, ""), `${object.attachment.displayName} · 第 7 页 · line 2-3:4-5`,
  ]);
});
it("shows note titles and block-in labels without block IDs or block-relative line coordinates", () => {
  const object = { kind: "note" as const, snapshot };
  expect(sourceLinkHint(object)).toEqual(["实验笔记"]);
  expect(sourceLinkHint(object, { kind: "block", blockId: "private-block-id", range })).toEqual(["block in 实验笔记"]);
  expect(sourceLinkHint({ kind: "note", snapshot: { ...snapshot, page: { ...page, title: "" } } })).toEqual(["未命名笔记"]);
});
it("uses the manuscript title and relative file path without an internal project ID", () => {
  const manuscript = { id: "private-project-id", title: "论文草稿", rootPath: parseManuscriptPath("main.tex"), status: "active" as const, createdAt: "now", updatedAt: "now" };
  const file = { manuscriptId: manuscript.id, relativePath: parseManuscriptPath("sections/intro.tex"), kind: "tex" as const, sizeBytes: 1, sha256: "hash", modifiedAt: "now" };
  expect(sourceLinkHint({ kind: "manuscript", manuscript })).toEqual(["写作项目：论文草稿"]);
  expect(sourceLinkHint({ kind: "manuscriptFile", manuscript, file }, { kind: "line", range })).toEqual(["写作项目：论文草稿", "sections/intro.tex · line 2-3:4-5"]);
});
