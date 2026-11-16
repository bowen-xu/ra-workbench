// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { expect, it, vi } from "vitest";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { WorkspaceTabBar, WorkspaceTabsProvider } from "../../../../../../apps/web/src/core/components/workspace-tabs/WorkspaceTabs";
import { useTextDocuments } from "../../../../../../apps/web/src/core/components/text-editor/useTextDocuments";
import { OPEN_TEXT_DOCUMENT } from "../../../../../../apps/web/src/core/components/text-editor/textDocumentEvents";
import { DeveloperModeProvider } from "../../../../../../apps/web/src/core/features/settings/DeveloperMode";
import { DoclingDeveloperActions } from "../../../../../../../plugins/ra-literature-library/src/features/ai/DoclingDeveloperActions";
import { panelItem } from "../../../../../../../plugins/ra-literature-library/src/features/ai/paperOverviewTestFixtures";
import type { DoclingGateway, DoclingParseRunDetail } from "../../../../../../apps/web/src/core/services/doclingGateway";
import { runDoclingParse } from "../../../../../../apps/web/src/core/services/doclingParseTask";
import { notify } from "../../../../../../apps/web/src/core/services/notifications";

vi.mock("../../../../../../apps/web/src/core/services/doclingParseTask", () => ({ runDoclingParse: vi.fn() }));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { error: vi.fn(), info: vi.fn() } }));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);

function detail(): DoclingParseRunDetail {
  return { id: "latest", attachmentId: "attachment", finishedAt: "2026-10-20", parseStatus: "success",
    jsonContent: { separateDocumentMustNotBeShown: true },
    rawResponse: { status: "success", processing_time: 12, errors: [], runtime: { engine: "docling-rs" },
      document: { json_content: { pictures: [{ image: { uri: "data:image/png;base64,AAABBB==" } }] } } },
  } as unknown as DoclingParseRunDetail;
}

async function harness(gateway: DoclingGateway, enabled = true) {
  const values = new Map<string, string>([["research-assistant.developer-mode", String(enabled)]]);
  selectAppStorage({ get length() { return values.size; }, key: i => [...values.keys()][i] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear() });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Shell({ item }: { readonly item: LiteratureItem }) {
    const text = useTextDocuments("library");
    return <><WorkspaceTabBar owner="library" textActive={text.activeId !== undefined} navigate={(_, __, ready) => { ready(); return true; }} />
      <DoclingDeveloperActions item={item} gateway={gateway} />{text.surface}</>;
  }
  const render = async (item = panelItem()) => { await act(async () => root.render(<WorkspaceTabsProvider><DeveloperModeProvider><Shell item={item} /></DeveloperModeProvider></WorkspaceTabsProvider>)); };
  await render();
  return { host, render, view: async () => { await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="查看最新解析数据"]')!.click()); },
    dispose: async () => { await act(async () => root.unmount()); host.remove(); } };
}

it("keeps developer controls and parse reads out of ordinary reading mode", async () => {
  const listParseRuns = vi.fn(async () => []);
  const h = await harness({ listParseRuns } as unknown as DoclingGateway, false);
  try {
    expect(h.host.querySelector('[aria-label="解析 PDF"]')).toBeNull();
    expect(h.host.querySelector('[aria-label="查看最新解析数据"]')).toBeNull();
    expect(listParseRuns).not.toHaveBeenCalled();
  } finally { await h.dispose(); }
});

it("opens complete rawResponse in the real read-only text tab and reuses its parse identity", async () => {
  const result = detail();
  const getParseRun = vi.fn(async () => result);
  const listParseRuns = vi.fn(async () => [{ id: "old", finishedAt: "2026-10-01" }, result]);
  const h = await harness({ listParseRuns, getParseRun } as unknown as DoclingGateway);
  try {
    expect(listParseRuns).not.toHaveBeenCalled();
    await h.view();
    expect(getParseRun).toHaveBeenCalledExactlyOnceWith("latest");
    const editor = EditorView.findFromDOM(h.host.querySelector<HTMLElement>(".cm-editor")!)!;
    expect(JSON.parse(editor.state.doc.toString())).toEqual(result.rawResponse);
    expect(editor.state.doc.toString()).toContain("data:image/png;base64,AAABBB==");
    expect(editor.state.doc.toString()).not.toContain("separateDocumentMustNotBeShown");
    expect(editor.state.facet(EditorState.readOnly)).toBe(true);
    expect(h.host.querySelector('.cm-content')?.getAttribute("contenteditable")).toBe("false");
    expect(h.host.querySelector('[role="tab"]')?.textContent).toContain("paper.docling.json");
    expect(document.querySelector('[role="dialog"][aria-label="最新 PDF 解析数据"]')).toBeNull();
    await h.view();
    expect(h.host.querySelectorAll('[role="tab"]')).toHaveLength(1);
    expect(EditorView.findFromDOM(h.host.querySelector<HTMLElement>(".cm-editor")!)).toBe(editor);
  } finally { await h.dispose(); }
});

it("finishes parsing without automatically opening a document, then opens it on demand", async () => {
  const result = detail();
  vi.mocked(runDoclingParse).mockResolvedValueOnce(result);
  const listParseRuns = vi.fn(async () => [result]);
  const h = await harness({ listParseRuns, getParseRun: vi.fn(async () => result) } as unknown as DoclingGateway);
  const changed = vi.fn(); window.addEventListener("research-assistant:paper-reading-changed", changed);
  try {
    await act(async () => h.host.querySelector<HTMLButtonElement>('[aria-label="解析 PDF"]')!.click());
    const attachment = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button => button.textContent === "paper.pdf")!;
    expect(attachment).toBeDefined();
    await act(async () => attachment.click());
    expect(runDoclingParse).toHaveBeenCalledWith(expect.anything(), "paper", "attachment", { paperTitle: "Paper", fileName: "paper.pdf" });
    expect(changed).toHaveBeenCalledOnce();
    expect(h.host.querySelector('[role="tab"]')).toBeNull();
    expect(listParseRuns).not.toHaveBeenCalled();
    expect(h.host.querySelector<HTMLButtonElement>('[aria-label="查看最新解析数据"]')!.disabled).toBe(false);
    await h.view();
    expect(h.host.querySelector('[role="tab"]')).not.toBeNull();
  } finally { window.removeEventListener("research-assistant:paper-reading-changed", changed); await h.dispose(); }
});

it("reports an empty result or a read failure without opening an empty file", async () => {
  const listParseRuns = vi.fn<DoclingGateway["listParseRuns"]>().mockResolvedValueOnce([])
    .mockRejectedValueOnce(new Error("[request_failed] synthetic read failure"));
  const h = await harness({ listParseRuns } as unknown as DoclingGateway);
  try {
    await h.view();
    expect(notify.info).toHaveBeenCalledWith("尚无解析数据");
    await h.view();
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("[request_failed] synthetic read failure"));
    expect(h.host.querySelector('[role="tab"]')).toBeNull();
    expect(h.host.querySelector<HTMLButtonElement>('[aria-label="查看最新解析数据"]')!.disabled).toBe(false);
  } finally { await h.dispose(); }
});

it("ignores a delayed read after the user switches literature items", async () => {
  let finish!: (value: DoclingParseRunDetail) => void;
  const response = new Promise<DoclingParseRunDetail>(resolve => { finish = resolve; });
  const getParseRun = vi.fn(() => response);
  const h = await harness({ listParseRuns: vi.fn(async () => [detail()]), getParseRun } as unknown as DoclingGateway);
  const opened = vi.fn(); window.addEventListener(OPEN_TEXT_DOCUMENT, opened);
  try {
    await h.view();
    expect(getParseRun).toHaveBeenCalledOnce();
    await h.render({ ...panelItem(), id: "another-paper" });
    await act(async () => finish(detail()));
    expect(opened).not.toHaveBeenCalled();
    expect(h.host.querySelector('[role="tab"]')).toBeNull();
  } finally { window.removeEventListener(OPEN_TEXT_DOCUMENT, opened); await h.dispose(); }
});
