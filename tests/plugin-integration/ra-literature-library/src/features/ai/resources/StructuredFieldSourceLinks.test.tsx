// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { overviewCandidate } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { StructuredFieldSourceLinks } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredFieldSourceLinks";
const navigation = vi.hoisted(() => ({inspect: vi.fn(async () => undefined), error: vi.fn()}));
vi.mock("../../../../../../../../plugins/ra-literature-library/src/features/ai/aiEvidenceNavigation", () => ({inspectAiReadingSource: navigation.inspect, readingSourceForAiEvidence: (value: unknown) => value}));
vi.mock("../../../../../../../apps/web/src/core/services/notifications", () => ({notify: {error: navigation.error}}));
(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
it("refreshes memoized source controls when evidence becomes stale without a body change", async () => {
  const candidate = overviewCandidate("memo-source", "paper");
  const resource: StructuredResource = {id: "reading", kind: "structuredReading", revision: 1,
    sourceUri: "source://library/paper/reading", title: "Reading", content: candidate.revisions[0]!.payload,
    reviewBaseline: null, pendingFieldIds: [], reviewedFieldIds: []};
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<StructuredFieldSourceLinks resource={resource} fieldId="research_question" evidence={candidate.evidenceBundle.items}/>));
    await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
    expect(navigation.inspect).toHaveBeenCalledOnce();
    const stale = candidate.evidenceBundle.items.map(source => ({...source, currentAnchorStatus: "stale" as const}));
    await act(async () => root.render(<StructuredFieldSourceLinks resource={{...resource, revision: 2}} fieldId="research_question" evidence={stale}/>));
    await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
    expect(navigation.inspect).toHaveBeenCalledOnce();
    expect(navigation.error).toHaveBeenCalledWith("来源已变化或不可用，请重新核对。");
  } finally {await act(async () => root.unmount()); host.remove();}
});
