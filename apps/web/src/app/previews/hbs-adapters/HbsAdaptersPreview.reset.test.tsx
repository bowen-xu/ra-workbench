// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HbsAdaptersPreview } from "./HbsAdaptersPreview";
import { HbsOperationLog } from "../../../core/components/hbs/operationLog";

const state = vi.hoisted(() => ({open: vi.fn(), reset: vi.fn()}));
vi.mock("./resetExample", () => ({resetExample: state.reset}));
vi.mock("./storage", () => ({openExample: state.open, previewCall: vi.fn()}));
vi.mock("../../../core/components/hbs/HbsNoteEditor", () => ({HbsNoteEditor: () => null}));
vi.mock("../../../core/components/hbs/HbsStructuredEditor", () => ({HbsStructuredEditor: () => null}));
vi.mock("../../../core/components/hbs/HbsAdapterWorkspace", () => ({HbsAdapterWorkspace: ({onReset}: {onReset: () => void}) => <button onClick={onReset}>重置当前资源</button>}));
let root: ReturnType<typeof createRoot>, host: HTMLDivElement;
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.clearAllMocks(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
it("keeps failure visible without reopening old checkpoints; retry only reopens after verified success", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const clear = vi.spyOn(HbsOperationLog.prototype, "clear");
  const getSnapshot = () => ({preserved: undefined});
  state.open.mockImplementation(async () => ({id: "resource", identity: {}, session: {adapter: {kind: "note"}, getSnapshot, subscribe: () => () => {}, dispose() {}},
    delivery: {getSnapshot: () => ({automatic: true}), subscribe: () => () => {}, setAutomatic() {}, dispose() {}},
    inspect: async () => ({version: 1})}));
  state.reset.mockRejectedValueOnce(new Error("MODEL_VERSION_CONFLICT: request reset-1"));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<HbsAdaptersPreview/>));
  await act(async () => host.querySelector<HTMLButtonElement>('button')!.click());
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("MODEL_RESET_FAILED：MODEL_VERSION_CONFLICT: request reset-1");
  expect(state.open).toHaveBeenCalledOnce();
  expect(host.textContent).toContain("重试重置当前资源");
  expect(clear).not.toHaveBeenCalled();
  state.reset.mockResolvedValueOnce(undefined);
  await act(async () => host.querySelector<HTMLButtonElement>('button')!.click());
  expect(state.open).toHaveBeenCalledTimes(2); expect(host.querySelector('[role="alert"]')).toBeNull();
  expect(clear).toHaveBeenCalledOnce();
  expect((clear.mock.contexts[0] as HbsOperationLog).getSnapshot()).toEqual([]);
});
