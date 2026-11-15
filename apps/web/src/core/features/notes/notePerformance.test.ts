import { describe, expect, it } from "vitest";
import {
  decodeNoteDocument,
  noteBlockId,
  notePageId,
  type NoteDocumentV1,
} from "@research-assistant/core/notes";

import { blockNoteToNoteDocument, noteDocumentToBlockNote } from "./editor/blockNoteAdapter";
import type { NoteEditorBlock } from "./editor/noteEditorSchema";
interface Sample { readonly p50: number; readonly p95: number }

function measure(operation: () => void, count = 20): Sample {
  for (let index = 0; index < 3; index += 1) operation();
  const samples = Array.from({ length: count }, () => {
    const startedAt = performance.now();
    operation();
    return performance.now() - startedAt;
  }).sort((left, right) => left - right);
  return { p50: samples[Math.floor(count * 0.5)]!, p95: samples[Math.floor(count * 0.95)]! };
}

function longDocument(): NoteDocumentV1 {
  return {
    schemaVersion: 1,
    pageId: notePageId("performance-page"),
    blocks: Array.from({ length: 2_000 }, (_, index) => ({
      id: noteBlockId(`performance-block-${index}`), type: "paragraph" as const, props: {},
      content: { kind: "inline" as const, items: [{ type: "text" as const, text: `第 ${index} 段固定性能夹具 `.repeat(4), styles: [] }] },
      children: [],
    })),
  };
}

describe("note P0 performance budgets", () => {
  it("keeps long-document decode and editor adaptation within frozen P95 budgets", () => {
    const document = longDocument();
    const decoder = measure(() => { decodeNoteDocument(document); });
    const adapter = measure(() => { blockNoteToNoteDocument(document.pageId, noteDocumentToBlockNote(document) as NoteEditorBlock[]); }, 10);
    console.info(`[notes-performance] decoder=${JSON.stringify(decoder)} adapter=${JSON.stringify(adapter)}`);
    expect(decoder.p95).toBeLessThan(100);
    expect(adapter.p95).toBeLessThan(250);
  });

  // The 4000-record persistent search budget runs in a real browser:
  // tests/ui/note-object-search.pw.ts. A memory-record mock cannot measure IDB.
});
