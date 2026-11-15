// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { notePageId, type NotePage } from "@research-assistant/core/notes";
import { notify } from "../../../services/notifications";
import { useNoteLifecycleNotice } from "./useNoteLifecycleNotice";

vi.mock("../../../services/notifications", () => ({ notify: { info: vi.fn() } }));
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
afterEach(async () => { if (root) await act(async () => root!.unmount()); host?.remove(); vi.clearAllMocks(); vi.unstubAllGlobals(); });
function View({ page, visible }: { page?: NotePage; visible: boolean }) { useNoteLifecycleNotice(page, visible); return null; }
const page: NotePage = { id: notePageId("trashed"), title: "研究计划", schemaVersion: 1, status: "trashed", revision: 1,
  position: 0, isFavorite: false, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" };

it("does not announce trash visits, while retaining the archive notice", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const render = async (visible: boolean, value = page) => act(async () => root!.render(<View page={value} visible={visible} />));
  await render(false); expect(notify.info).not.toHaveBeenCalled();
  await render(true);
  expect(notify.info).not.toHaveBeenCalled();
  await render(true, { ...page, revision: 2 }); expect(notify.info).not.toHaveBeenCalled();
  await render(false); await render(true); expect(notify.info).not.toHaveBeenCalled();
  await render(true, { ...page, status: "active" }); expect(notify.info).not.toHaveBeenCalled();
  await render(true, { ...page, status: "archived" });
  expect(notify.info).toHaveBeenLastCalledWith("笔记已归档，当前内容只读。", { id: "note-lifecycle:trashed" });
});
