// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { AgentContextSelection, AgentContextState } from "@research-assistant/core/agent";
import { READING_TEMPLATE_CATALOG_ID, type StructuredResource } from "@research-assistant/core/structured-resources";
import { mockStructuredConversationContext } from "../../../../app/runtime/mock/mockStructuredContext";
import { selectedCatalogItem } from "./contextCatalogSelection";
import { ContextCatalogRow } from "./ContextCatalogRow";
import { overviewCandidate, panelItem } from "../../../../../../../plugins/literature_library/src/features/ai/paperOverviewTestFixtures";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const original = panelItem();
const paper = {...original, attachments: original.attachments.map(file => ({...file, id: "pdf", displayName: "source.pdf"}))};
const reading: StructuredResource = {
  id: "reading", kind: "structuredReading", paperId: paper.id, title: "结构化阅读",
  sourceUri: "source://library/paper/reading", revision: 1, content: null, reviewBaseline: null,
  pendingFieldIds: [], reviewedFieldIds: [], templateRef: {resourceId: "template", revision: 2},
};
const template: StructuredResource = {...reading, id: "template", kind: "readingTemplate", title: "本文阅读模板",
  sourceUri: "source://library/paper/template", revision: 2};
const evidence = overviewCandidate("source", paper.id).evidenceBundle.items.map(source => ({...source, attachmentId: "pdf"}));
const inventory: AgentContextSelection = {label: "Paper", resource: {kind: "paperGroup", resourceId: "paper", parts: [
  {label: "source.pdf", resource: {kind: "paper", resourceId: "paper", attachmentId: "pdf"}, selected: true, access: "readOnly"},
  {label: "结构化阅读", resource: {kind: "structuredObject", resourceId: "reading"}, selected: true, access: "readWrite"},
  {label: "本文阅读模板", resource: {kind: "structuredObject", resourceId: "template"}, selected: true, access: "readWrite"},
]}};

it.each([
  {separate: false, writable: true},
  {separate: true, writable: false},
])("preserves own-template authority with separate read-only selection=$separate", async ({separate, writable}) => {
  const context = mockStructuredConversationContext(paper, reading, evidence, template);
  let state: AgentContextState = context;
  if (separate) {
    const group = context.persistent[0]!;
    if (group.resource.kind !== "paperGroup") throw new Error("fixture");
    state = {...context, persistent: [
      {...group, resource: {...group.resource, parts: group.resource.parts.filter(part => part.resource.resourceId !== template.id)}},
      {label: template.title, resource: {kind: "structuredObject", resourceId: template.id, access: "readOnly"}},
    ]};
  }
  const selected = selectedCatalogItem(inventory, state);
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  try {
    await act(async () => root.render(<ul><ContextCatalogRow item={inventory} selected={selected!}
      disabled={false} onChange={() => undefined}/></ul>));
    const input = (label: string): HTMLInputElement => host.querySelector(`[aria-label="${label}"]`)!;
    expect(input("使用 Paper · 本文阅读模板").checked).toBe(true);
    expect(input("Paper · 本文阅读模板 权限").checked).toBe(writable);
    expect(input("Paper · 结构化阅读 权限").checked).toBe(true);
    expect(input("使用 Paper · source.pdf").checked).toBe(true);
    expect(input("Paper · source.pdf 权限").checked).toBe(false);
  } finally {
    await act(async () => root.unmount());
    host.remove();
  }
});

it("keeps a pinned global template as a separate read-only selection", () => {
  const {paperId: _, ...global} = template;
  const context = mockStructuredConversationContext(paper, reading, evidence, global);
  expect(context.persistent).toHaveLength(3);
  expect(context.persistent[1]).toMatchObject({resource: {
    kind: "structuredObject", resourceId: template.id, access: "readOnly",
  }});
  expect(context.persistent[0]).toMatchObject({resource: {parts: [
    {access: "readOnly"}, {access: "readWrite"},
  ]}});
  expect(context.persistent[2]).toMatchObject({resource: {
    kind: "templateCatalog", resourceId: READING_TEMPLATE_CATALOG_ID, access: "readOnly",
  }});
});
