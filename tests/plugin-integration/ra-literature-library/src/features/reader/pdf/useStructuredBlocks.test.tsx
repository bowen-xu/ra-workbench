/** @vitest-environment happy-dom */
import { act, StrictMode, useCallback, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DoclingGateway, DoclingParseRunDetail } from "../../../../../../../apps/web/src/core/services/doclingGateway";
import { notify } from "@research-assistant/workbench-sdk/feedback";
import { StructuredBlocksToggle } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/pdf/StructuredBlocksToggle";
import { useStructuredBlocks } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/pdf/useStructuredBlocks";

vi.mock("../../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { warning: vi.fn() } }));
const parseAttachment = vi.fn<DoclingGateway["parseAttachment"]>();
const gateway = { parseAttachment };
const detail = { id: "run", parseStatus: "success" } as DoclingParseRunDetail;
let root: ReturnType<typeof createRoot>, host: HTMLDivElement;
let control: ReturnType<typeof useStructuredBlocks>;
const changes = vi.fn();

function Harness({ attachmentId = "pdf", restored = false }: { attachmentId?: string; restored?: boolean }) {
  const [open, setOpen] = useState(restored);
  const changeOpen = useCallback((value: boolean) => { changes(attachmentId, value); setOpen(value); }, [attachmentId]);
  control = useStructuredBlocks({ gateway, itemId: "paper", attachmentId, contentSha256: "hash", open, setOpen: changeOpen });
  return <StructuredBlocksToggle enabled={open} preparing={control.preparing} onToggle={control.toggle} />;
}

async function render(props: Parameters<typeof Harness>[0] = {}) {
  await act(async () => root.render(<StrictMode><Harness {...props} /></StrictMode>));
}
function button() { return host.querySelector("button")!; }
function pending() {
  let resolve!: (value: DoclingParseRunDetail) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<DoclingParseRunDetail>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.clearAllMocks();
  parseAttachment.mockReset();
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("prepares once even before disabled commits, then shows blocks and supports hiding without parsing", async () => {
  const task = pending(); parseAttachment.mockReturnValue(task.promise);
  await render();
  await act(async () => { control.toggle(); control.toggle(); });
  expect(parseAttachment).toHaveBeenCalledExactlyOnceWith("paper", "pdf", { force: false });
  expect(button().disabled).toBe(true);
  expect(button().getAttribute("aria-busy")).toBe("true");
  expect(button().querySelector(".spinning")).not.toBeNull();
  await act(async () => task.resolve(detail));
  expect(button().disabled).toBe(false);
  expect(button().getAttribute("aria-pressed")).toBe("true");
  await act(async () => button().click());
  expect(button().getAttribute("aria-pressed")).toBe("false");
  expect(parseAttachment).toHaveBeenCalledTimes(1);
});

it.each([
  { code: "cancelled", message: "解析已终止" },
  { code: "response_timeout", message: "任务 task-123 状态不可用" },
  new Error("未知原因"),
])("unlocks after cancellation or failure without automatic retries: %s", async reason => {
  const task = pending(); parseAttachment.mockReturnValueOnce(task.promise).mockResolvedValue(detail);
  await render(); await act(async () => button().click());
  expect(button().disabled).toBe(true);
  await act(async () => task.reject(reason));
  expect(button().disabled).toBe(false);
  expect(button().getAttribute("aria-pressed")).toBe("false");
  expect(parseAttachment).toHaveBeenCalledTimes(1);
  expect(notify.warning).toHaveBeenCalledWith(expect.stringContaining(reason.message));
  await act(async () => button().click());
  expect(parseAttachment).toHaveBeenCalledTimes(2);
  expect(button().getAttribute("aria-pressed")).toBe("true");
});

it("uses cached success without forcing a new parse", async () => {
  parseAttachment.mockResolvedValue(detail);
  await render(); await act(async () => button().click());
  expect(parseAttachment).toHaveBeenCalledExactlyOnceWith("paper", "pdf", { force: false });
  expect(button().disabled).toBe(false);
  expect(button().getAttribute("aria-pressed")).toBe("true");
});

it("prepares a missing restored overlay once while multiple pages report it", async () => {
  const task = pending(); parseAttachment.mockReturnValue(task.promise);
  await render({ restored: true });
  const missing = { code: "not_found", message: "结构化结果缺失" };
  await act(async () => { control.prepareMissing(missing); control.prepareMissing(missing); });
  expect(button().getAttribute("aria-pressed")).toBe("false");
  expect(button().disabled).toBe(true);
  expect(parseAttachment).toHaveBeenCalledTimes(1);
  await act(async () => task.resolve(detail));
  expect(button().getAttribute("aria-pressed")).toBe("true");
  await act(async () => control.prepareMissing(missing));
  expect(button().disabled).toBe(false);
  expect(button().getAttribute("aria-pressed")).toBe("false");
  expect(parseAttachment).toHaveBeenCalledTimes(1);
  expect(notify.warning).toHaveBeenCalledWith(expect.stringContaining("[not_found]"));
});

it.each(["complete", "fail"])("does not apply an old attachment's %s to a new attachment", async outcome => {
  const task = pending(); parseAttachment.mockReturnValue(task.promise);
  await render(); await act(async () => button().click());
  await render({ attachmentId: "another" });
  expect(button().disabled).toBe(false);
  changes.mockClear();
  await act(async () => outcome === "complete" ? task.resolve(detail) : task.reject(new Error("old task")));
  expect(changes).not.toHaveBeenCalled();
  expect(notify.warning).not.toHaveBeenCalled();
  expect(button().getAttribute("aria-pressed")).toBe("false");
});
