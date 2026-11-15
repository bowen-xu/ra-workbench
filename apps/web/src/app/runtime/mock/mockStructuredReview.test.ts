import { expect, it } from "vitest";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { adoptMockStructuredTemplate } from "./mockStructuredReview";

function resource(kind: StructuredResource["kind"], content: unknown): StructuredResource {
  return {id: kind, kind, title: kind, sourceUri: `source://library/paper/${kind}`, paperId: "paper",
    revision: 1, content, reviewBaseline: content, pendingFieldIds: [], reviewedFieldIds: []};
}

it("keeps excluded field bodies on template change and restores them by stable ID", () => {
  const old = {field: "old", status: "filled", statements: [{id: "old-body", text: "preserved"}]};
  const reading = resource("structuredReading", {kind: "paper_overview_unit_v1", schemaVersion: 1,
    fields: [{field: "current", status: "not_found", statements: []}, old]});
  const first = resource("readingTemplate", {fields: [{id: "current", title: "Current", question: "Why?"}]});
  const adopted = adoptMockStructuredTemplate(reading, first);
  expect(adopted.content).toMatchObject({retainedFields: [old]});
  expect(adopted.pendingFieldIds).toContain("$retained:old");
  const restored = adoptMockStructuredTemplate(adopted, {...first, revision: 2,
    content: {fields: [{id: "old", title: "Old", question: "Restored?"}]}});
  expect(restored.content).toMatchObject({fields: [old]});
});
