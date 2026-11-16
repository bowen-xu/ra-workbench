// @vitest-environment happy-dom
import { notify } from "@research-assistant/workbench-sdk/feedback";
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn(), success: vi.fn(), warning: vi.fn() } }));
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DoclingGateway } from "../../../../../../apps/web/src/core/services/doclingGateway";
import { panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import { ReferenceVerificationPanel } from "../../../../../../../plugins/ra-literature-library/src/features/synthesis/ReferenceVerificationPanel";
const state = vi.hoisted(() => ({ list: vi.fn() }));
vi.mock("../../../../../../../plugins/ra-literature-library/src/services/referenceRecordGateway", () => ({ getReferenceRecordGateway: () => ({ list: state.list }) }));
vi.mock("../../../../../../../plugins/ra-literature-library/src/services/literatureGateway", () => ({ getLiteratureGateway: () => ({}) }));
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.clearAllMocks(); state.list.mockResolvedValue([]); host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function toggle(open: boolean) {
  await act(async () => {
    const details = host.querySelector("details")!;
    details.open = open;
    details.dispatchEvent(new Event("toggle"));
  });
}
it("defers detection until expanded and labels an actually checked empty bibliography", async () => {
  const gateway = { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway;
  const settled = vi.fn();
  await act(async () => root.render(<ReferenceVerificationPanel item={panelItem()} doclingGateway={gateway} onOpenAttachment={vi.fn()} onInitialLoadSettled={settled} />));
  expect(host.querySelector("summary")?.textContent).toBe("参考文献");
  expect(gateway.listParseRuns).not.toHaveBeenCalled();
  expect(settled).toHaveBeenLastCalledWith(true);
  await toggle(true);
  expect(host.querySelector("summary")?.textContent).toBe("参考文献（未识别）");
  expect(gateway.listParseRuns).toHaveBeenCalledExactlyOnceWith(panelItem().id);
  expect(host.querySelectorAll("button")).toHaveLength(0);
  expect(host.textContent).not.toContain("完成 PDF 解析后");
});
it("keeps saved records visible and does not label them unidentified", async () => {
  state.list.mockResolvedValue([{ id: "reference", revision: 1, rawText: "Synthetic reference", fields: [], provenanceKind: "human", parseState: "unparsed", identityState: "unverified", semanticState: "unreviewed" }]);
  const gateway = { listParseRuns: vi.fn(async () => []) } as unknown as DoclingGateway;
  await act(async () => root.render(<ReferenceVerificationPanel item={panelItem()} doclingGateway={gateway} onOpenAttachment={vi.fn()} />));
  expect(host.querySelector("summary")?.textContent).toBe("参考文献");
  expect(host.querySelector("q")?.textContent).toBe("Synthetic reference");
  expect(host.querySelector("button")?.textContent).toBe("编辑／核验");
});
it("reports parsing errors without presenting them as an empty bibliography", async () => {
  const gateway = { listParseRuns: vi.fn(async () => { throw new Error("parse unavailable"); }) } as unknown as DoclingGateway;
  await act(async () => root.render(<ReferenceVerificationPanel item={panelItem()} doclingGateway={gateway} onOpenAttachment={vi.fn()} />));
  expect(notify.error).not.toHaveBeenCalled();
  await toggle(true);
  expect(host.querySelector("summary")?.textContent).toBe("参考文献");
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("parse unavailable"));
});

it("reads the selected parse only on demand and retains it after collapsing", async () => {
  const item = panelItem(), attachment = item.attachments[0]!;
  const gateway = {
    listParseRuns: vi.fn(async () => [{id:"parse",attachmentId:attachment.id,contentSha256:attachment.contentSha256,parseStatus:"success",finishedAt:"2026-10-24"}]),
    getParseRun: vi.fn(async () => ({jsonContent:{texts:[{self_ref:"#/texts/0",label:"reference",text:"Synthetic reference",prov:[{page_no:2}]}]}})),
  } as unknown as DoclingGateway;
  await act(async () => root.render(<StrictMode><ReferenceVerificationPanel item={item} doclingGateway={gateway} onOpenAttachment={vi.fn()} /></StrictMode>));
  expect(gateway.getParseRun).not.toHaveBeenCalled();
  await toggle(true);
  expect(gateway.getParseRun).toHaveBeenCalledExactlyOnceWith("parse");
  expect(host.querySelector("q")?.textContent).toBe("Synthetic reference");
  await toggle(false); await toggle(true);
  expect(gateway.getParseRun).toHaveBeenCalledTimes(1);
  // StrictMode may repeat the small header read, but never the document read.
  expect(gateway.listParseRuns).toHaveBeenCalledTimes(2);
});
