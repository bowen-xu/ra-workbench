// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { EMPTY_AGENT_CONTEXT } from "@research-assistant/core/agent";
import { readStructuredStartupPreferences } from "./structuredStartupPreferences";
import { createTestNamespacedStorage } from "./testing/createTestNamespacedStorage";

function storage(): Storage { const data = new Map<string, string>(); return {
  get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
  getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); },
  removeItem: key => { data.delete(key); }, clear: () => data.clear(),
}; }

it("collects every paper's mixed selections and both context lifetimes without opening paper views", () => {
  const saved = storage();
  saved.setItem("research-assistant.reading-presentation.v1:paper-b", JSON.stringify({fields: [
    {candidateId: "older", fieldId: "question"}, {candidateId: "newer", fieldId: "result"},
  ], hiddenUnitIds: ["hidden"], hiddenResourceVersions: [{resourceId: "reading", revision: 2}]}));
  saved.setItem("research-assistant.reading-presentation.v1:paper-a", JSON.stringify({fields: [], hiddenUnitIds: []}));
  saved.setItem("research-assistant:agent-context:v1:conversation", JSON.stringify({...EMPTY_AGENT_CONTEXT,
    temporary: [{label: "This turn", resource: {kind: "paper", resourceId: "paper-b"}}]}));
  const draft = readStructuredStartupPreferences(saved);
  expect(draft.presentations.map(value => value.paperId)).toEqual(["paper-a", "paper-b"]);
  expect(draft.presentations[1]?.presentation.fields).toEqual([
    {candidateId: "older", fieldId: "question"}, {candidateId: "newer", fieldId: "result"},
  ]);
  expect(draft.contexts[0]?.state).toMatchObject({persistent: [], temporary: [{label: "This turn"}]});
});

it.each(["{", JSON.stringify({fields: [{candidateId: "", fieldId: "q"}], hiddenUnitIds: []}),
  JSON.stringify({fields: [], hiddenUnitIds: [], hiddenResourceVersions: [{resourceId: "r", revision: -1}]})])(
  "blocks damaged paper preferences without modifying their original value: %s", raw => {
    const saved = storage(), key = "research-assistant.reading-presentation.v1:paper";
    saved.setItem(key, raw);
    expect(() => readStructuredStartupPreferences(saved)).toThrow(key);
    expect(saved.getItem(key)).toBe(raw);
  },
);

it("uses the runtime's isolated preview namespace during enumeration", () => {
  const backing = storage(), preview = createTestNamespacedStorage(backing);
  backing.setItem("research-assistant.reading-presentation.v1:private", "invalid business data");
  preview.setItem("research-assistant.reading-presentation.v1:preview", JSON.stringify({fields: [], hiddenUnitIds: []}));
  expect(readStructuredStartupPreferences(preview).presentations.map(value => value.paperId)).toEqual(["preview"]);
});
