// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { DEFAULT_PORTRAIT_TEMPLATE, PORTRAIT_DIMENSIONS, type AiCandidate } from "@research-assistant/core/ai";
import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { overviewCandidate, panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { PaperPortraitPanel } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PaperPortraitPanel";

vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn() } }));
function deferred() {
  let resolve!: (value: readonly AiCandidate[]) => void;
  const promise = new Promise<readonly AiCandidate[]>(done => { resolve = done; });
  return { promise, resolve };
}
function portrait(paperId: string): AiCandidate {
  const base = overviewCandidate(paperId, paperId);
  return { ...base, candidateKind: "paper_portrait_v1", revisions: [{ ...base.revisions[0]!, payload: {
    kind: "paper_portrait_v1", schemaVersion: 1, coverage: "测试摘录",
    claims: PORTRAIT_DIMENSIONS.map((dimension, index) => ({ id: `claim_${index}`, dimension,
      label: `分类-${paperId}`, description: "测试分类", certainty: "uncertain", evidence: [] })),
    template: DEFAULT_PORTRAIT_TEMPLATE,
  } }] };
}

it("shares StrictMode reads across rails and ignores a previous paper's late response", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  selectAppStorage(localStorage); localStorage.clear();
  const first = deferred(), next = deferred();
  const listCandidates = vi.fn((_projectId?: string, scope?: { paperId: string }) => scope?.paperId === "first" ? first.promise : next.promise);
  const gateway = { listCandidates } as unknown as AiAssistanceGateway;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const render = (paperId: string) => act(async () => root.render(<StrictMode>
    <PaperPortraitPanel item={{ ...panelItem(), id: paperId }} gateway={gateway}/>
    <PaperPortraitPanel item={{ ...panelItem(), id: paperId }} gateway={gateway}/>
  </StrictMode>));
  try {
    await render("first");
    expect(listCandidates).toHaveBeenCalledExactlyOnceWith(undefined, { paperId: "first" });
    await render("next");
    expect(listCandidates).toHaveBeenCalledTimes(2);
    await act(async () => next.resolve([portrait("next")]));
    expect(host.textContent).toContain("分类-next");
    await act(async () => first.resolve([portrait("first")]));
    expect(host.textContent).toContain("分类-next");
    expect(host.textContent).not.toContain("分类-first");
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});
