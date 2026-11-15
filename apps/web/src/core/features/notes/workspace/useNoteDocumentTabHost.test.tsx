// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { navigateSource } from "../../agent/sourceNavigation";
import { openNoteDocumentTab } from "./openNoteDocumentTab";
import { useNoteDocumentTabHost } from "./useNoteDocumentTabHost";

vi.mock("../../agent/sourceNavigation", () => ({ navigateSource: vi.fn(async () => undefined) }));
let root: ReturnType<typeof createRoot> | undefined;
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = undefined; vi.clearAllMocks(); vi.unstubAllGlobals(); });

async function setup(chooseView: Parameters<typeof useNoteDocumentTabHost>[0]["chooseView"]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const openPage = vi.fn();
  function Host() { useNoteDocumentTabHost({ chooseView, openPage }); return null; }
  root = createRoot(document.createElement("div"));
  await act(async () => root!.render(<Host />));
  return openPage;
}

it("releases the page window only after the shared navigation/save guard accepts a document tab", async () => {
  let accepted: (() => void) | undefined;
  const choose = vi.fn<Parameters<typeof useNoteDocumentTabHost>[0]["chooseView"]>((_owner, _id, ready) => { accepted = ready; return true; });
  const openPage = await setup(choose), release = vi.fn();
  const opened = openNoteDocumentTab({kind: "page", pageId: "canonical-page"}, release);
  expect(choose).toHaveBeenCalledWith("notes", "canonical-page", expect.any(Function), "document", expect.any(Function));
  expect(release).not.toHaveBeenCalled(); expect(openPage).not.toHaveBeenCalled();
  await act(async () => { accepted?.(); await opened; });
  expect(openPage).toHaveBeenCalledExactlyOnceWith("canonical-page"); expect(release).toHaveBeenCalledOnce();
});

it("routes writing windows through the same protocol and document presentation, preserving the file identity", async () => {
  const choose = vi.fn(() => true), release = vi.fn(); await setup(choose);
  await openNoteDocumentTab({kind: "writing", manuscriptId: "project one", path: "notes/行文 思路.ranote"}, release);
  expect(navigateSource).toHaveBeenCalledExactlyOnceWith("source://writings/project%20one/notes/%E8%A1%8C%E6%96%87%20%E6%80%9D%E8%B7%AF.ranote", {presentation: "document"});
  expect(choose).not.toHaveBeenCalled(); expect(release).toHaveBeenCalledOnce();
});

it("keeps the floating owner for both immediate and asynchronous navigation refusal", async () => {
  let cancel: (() => void) | undefined;
  const choose = vi.fn<Parameters<typeof useNoteDocumentTabHost>[0]["chooseView"]>((_owner, _id, _ready, _presentation, cancelled) => { cancel = cancelled; return false; });
  const openPage = await setup(choose), release = vi.fn();
  await expect(openNoteDocumentTab({kind: "page", pageId: "page"}, release)).rejects.toThrow("无法打开");
  choose.mockImplementation((_owner, _id, _ready, _presentation, cancelled) => { cancel = cancelled; return true; });
  const pending = openNoteDocumentTab({kind: "page", pageId: "page"}, release);
  const failure = expect(pending).rejects.toThrow("无法打开"); cancel?.(); await failure;
  expect(openPage).not.toHaveBeenCalled(); expect(release).not.toHaveBeenCalled();
});

it("preserves writing windows on an original source-navigation failure and rejects without an app host", async () => {
  await setup(() => true); const release = vi.fn();
  vi.mocked(navigateSource).mockRejectedValueOnce(new Error("file_revision_conflict: disk changed"));
  await expect(openNoteDocumentTab({kind: "writing", manuscriptId: "project", path: "notes/note.ranote"}, release)).rejects.toThrow("file_revision_conflict");
  expect(release).not.toHaveBeenCalled();
  await act(async () => root!.unmount()); root = undefined;
  await expect(openNoteDocumentTab({kind: "page", pageId: "page"}, release)).rejects.toThrow("未接入");
});

it("rejects an in-flight transfer when the app host unmounts and ignores a late acceptance", async () => {
  let accepted: (() => void) | undefined;
  const openPage = await setup((_owner, _id, ready) => { accepted = ready; return true; }), release = vi.fn();
  const pending = openNoteDocumentTab({kind: "page", pageId: "page"}, release);
  const failure = expect(pending).rejects.toThrow("宿主已关闭");
  await act(async () => root!.unmount()); root = undefined; await failure;
  accepted?.(); expect(openPage).not.toHaveBeenCalled(); expect(release).not.toHaveBeenCalled();
});
