import { expect, it } from "vitest";
import { decodeMockState, encodeMockState } from "../../../core/runtime/mockStateCodec";
import { decodeStructuredState, encodeStructuredState } from "./mockStructuredStateCodec";

const resource = (revision: number, content: unknown) => ({id: "reading", kind: "structuredReading",
  sourceUri: "source://library/paper/reading", revision, content, reviewBaseline: content,
  draft: {content: '{invalid raw draft', revision: 2}});

it("deduplicates repeated documents without dropping versions, receipts or raw repair drafts", () => {
  const content = {fields: [{text: "完整正文与原文定位 🧪".repeat(500)}],
    content: 8, reviewBaseline: {format: "structured-resource-documents-v1", documents: ["opaque"]}};
  const rows = Array.from({length: 40}, (_, i) => resource(i, content));
  const state = {resources: [rows[39]], versions: rows, receipts: {one: {resource: rows[20]}},
    historyTransitions: [{before: rows[20], after: rows[21]}]};
  const packed = encodeStructuredState(state);
  expect(packed.length).toBeLessThan(encodeMockState(state).length / 10);
  expect(decodeStructuredState(packed)).toEqual(state);
  const reopened = decodeStructuredState<typeof state>(packed);
  expect(reopened.versions[0]!.content).not.toBe(reopened.versions[1]!.content);
});

it("reads old snapshots and preserves null, scalar and document-shaped payloads", () => {
  const values = [null, "raw text", 0, resource(99, {nested: true})];
  for (const content of values) {
    const state = {resources: [resource(1, content)]};
    const old = encodeMockState(state);
    expect(decodeStructuredState(old)).toEqual(decodeMockState(old));
    expect(decodeStructuredState(encodeStructuredState(state))).toEqual(state);
  }
});

it("rejects invalid document references rather than replacing saved content", () => {
  const encoded = JSON.parse(encodeStructuredState({resources: [resource(1, null)]}));
  encoded.state = JSON.stringify({resources: [resource(1, 999)]});
  expect(() => decodeStructuredState(JSON.stringify(encoded))).toThrow("正文引用损坏");
});
