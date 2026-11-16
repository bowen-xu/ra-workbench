// @vitest-environment happy-dom

import type { ReaderViewStateV2 } from "@research-assistant/core/reader";
import { noteBlockId } from "@research-assistant/core/notes";
import { beforeEach, describe, expect, it } from "vitest";
import {createRecoveryDraft,saveRecoveryDraft} from "../../../../../../../apps/web/src/core/services/recoveryDraftStore";

import {
  clearReaderRecoveryDraft,
  loadReaderRecoveryDraft,
  saveReaderRecoveryDraft,
  type ReaderRecoveryPayload,
} from "../../../../../../../../plugins/ra-literature-library/src/features/reader/session/readerRecoveryDraft";

class MemoryStorage implements Storage {
  readonly #values = new Map<string, string>();
  get length(): number { return this.#values.size; }
  clear(): void { this.#values.clear(); }
  getItem(key: string): string | null { return this.#values.get(key) ?? null; }
  key(index: number): string | null { return [...this.#values.keys()][index] ?? null; }
  removeItem(key: string): void { this.#values.delete(key); }
  setItem(key: string, value: string): void { this.#values.set(key, value); }
}

const hash = "a".repeat(64);
const readingState: ReaderViewStateV2 = {
  schemaVersion: 2,
  pageIndex: 3,
  scale: { kind: "page-width" },
  top: 120,
  left: 0,
  scrollMode: "vertical",
  spreadMode: "none",
};

const payload: ReaderRecoveryPayload = {
  pageNote: "尚未提交的页级笔记",
  selectionSessions: [{
    id: "selection-1",
    active: true,
    dirty: true,
    pending: {
      text: "selected text",
      prefix: "before",
      suffix: "after",
      pageIndex: 2,
      pageLabel: "3",
      rects: [{ x: 0.1, y: 0.2, width: 0.3, height: 0.04 }],
      segments: [{
        location: { startLine: 2, startColumn: 3, endLine: 3, endColumn: 5 },
        pageIndex: 2,
        pageLabel: "3",
        rects: [{ x: 0.1, y: 0.2, width: 0.3, height: 0.04 }],
        text: "selected text",
        prefix: "before",
        suffix: "after",
      }],
      popupPageIndex: 2,
      popupRect: { x: 0.1, y: 0.2, width: 0.3, height: 0.04 },
    },
    actionDraft: {
      mode: "actions",
      note: "my interpretation",
      referenceRawText: "selected text",
      conceptDirty: false,
      referenceDirty: false,
    },
  }],
  annotationDrafts: [{
    id: "note-1" as never,
    baseRevision: 4,
    position: { x: .3, y: .6, width: .00001, height: .00001 },
    body: "edited annotation",
    color: "#f4cf65",
    status: "active",
  }],
  readingState,
};

describe("reader recovery draft", () => {
  beforeEach(() => {
    Object.defineProperty(window, "localStorage", { configurable: true, value: new MemoryStorage() });
  });

  it("round-trips all user-authored reader state for the same attachment content", () => {
    saveReaderRecoveryDraft("attachment-1", hash, payload);
    expect(loadReaderRecoveryDraft("attachment-1", hash)).toMatchObject(payload);
    clearReaderRecoveryDraft("attachment-1");
    expect(loadReaderRecoveryDraft("attachment-1", hash)).toBeUndefined();
  });

  it("does not apply a draft to changed PDF bytes and preserves the old record", () => {
    saveReaderRecoveryDraft("attachment-1", hash, payload);
    expect(() => loadReaderRecoveryDraft("attachment-1", "b".repeat(64))).toThrow("PDF 内容已变化");
    expect(loadReaderRecoveryDraft("attachment-1", hash)).toMatchObject(payload);
  });
  it("does not persist annotation comment sessions as unfinished reader drafts", () => {
    const session = {...payload.selectionSessions[0]!, actionDraft: {...payload.selectionSessions[0]!.actionDraft!, mode:"annotation" as const, annotationComment:"为什么？🧪\n尚未发送", annotationEvidence:[{type:"annotation" as const,source:"source://library/paper/pdf:#line{1:1-1:1-4}",title:"原文",text:"选段",comment:""}]}};
    saveReaderRecoveryDraft("attachment-1", hash, {...payload, selectionSessions:[session]});
    expect(loadReaderRecoveryDraft("attachment-1",hash)?.selectionSessions).toEqual([]);
  });
  it("drops obsolete annotation reader drafts from an older recovery record",()=>{
    const obsolete={...payload.selectionSessions[0]!,actionDraft:{...payload.selectionSessions[0]!.actionDraft!,mode:"annotation",annotationComment:"旧评论"}};
    saveRecoveryDraft(createRecoveryDraft({scope:"pdf-reader",resourceId:"attachment-1",slot:"session"},{...payload,selectionSessions:[obsolete]},{hash}));
    expect(loadReaderRecoveryDraft("attachment-1",hash)?.selectionSessions).toEqual([]);
    expect(loadReaderRecoveryDraft("attachment-1",hash)?.annotationDrafts).toEqual(payload.annotationDrafts);
  });
  it("retains the existing annotation identity and color in a recovered selection form", () => {
    const session = { ...payload.selectionSessions[0]!, annotationId: noteBlockId("existing"), anchorId: "anchor", highlightColor: "#91d5a3" };
    saveReaderRecoveryDraft("attachment-1", hash, { ...payload, selectionSessions: [session] });
    expect(loadReaderRecoveryDraft("attachment-1", hash)?.selectionSessions[0]).toMatchObject(session);
  });
});
