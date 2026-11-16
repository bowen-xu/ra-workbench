// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteIndexItem } from "@research-assistant/core/notes";
import { NotesProvider, useNotesClient, type NotesClient } from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import { browserNotesGateway } from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";
import { selectAppStorage } from "../../../../../../../apps/web/src/core/runtime/appStorage";
import { NoteFavorites } from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/NoteFavorites";

afterEach(() => vi.unstubAllGlobals());
it("renders shared title-only links, opens originals and reorders through the common keyboard controls", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  window.localStorage.clear(); selectAppStorage(window.localStorage);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const items: NoteIndexItem[] = ["first", "second"].map(id => ({ id, title: id, snippet: "正文不应出现在收藏", category: "independent", updatedAt: "2026-10-22", status: "active", isFavorite: true, sourceLabel: "文献不应出现在收藏", target: { kind: "page", pageId: notePageId(id) } }));
  const open = vi.fn(), reorder = vi.fn(async () => {});
  try {
    await act(async () => root.render(<NotesProvider gateway={browserNotesGateway}><NoteFavorites items={items} selectedPageId={undefined} onOpen={open} onReorder={reorder} onAddInside={vi.fn()} onAction={vi.fn()} disabled={false} /></NotesProvider>));
    expect(host.querySelectorAll('.note-page-link-block.inline-reference[data-underline="none"]')).toHaveLength(2);
    expect(host.querySelector('strong')).toBeNull();
    expect([...host.querySelectorAll(".note-page-link-block")].map(el => el.textContent)).toEqual(["first", "second"]);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="打开笔记：first"]')!.click());
    expect(open).toHaveBeenCalledWith(items[0], false);
    await act(async () => host.querySelector('[aria-label="打开笔记：first"]')!.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowDown", altKey: true })));
    expect(reorder).toHaveBeenCalledWith(["second", "first"]);
    expect(host.querySelectorAll('[data-reorder-id]')).toHaveLength(2);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("only shows disclosures for block page links and removes them when the last link is removed", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear(); selectAppStorage(localStorage);
  const a = await browserNotesGateway.createPage({title: "parent"}), b = await browserNotesGateway.createPage({title: "leaf"});
  const item: NoteIndexItem = {id: `page:${a.page.id}`, title: "parent", snippet: "", category: "independent", updatedAt: "", status: "active", isFavorite: true, target: {kind: "page", pageId: a.page.id}};
  let client: NotesClient | undefined;
  function Favorites() {
    client = useNotesClient();
    return <NoteFavorites items={[item]} selectedPageId={undefined} onOpen={vi.fn()} onReorder={vi.fn()} onAddInside={vi.fn()} onAction={vi.fn()} disabled={false} />;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<NotesProvider gateway={browserNotesGateway}><Favorites /></NotesProvider>));
    expect(host.querySelector(".note-favorite-toggle")).toBeNull();
    expect(host.textContent).not.toContain("暂无页面链接");
    await act(async () => {
      await browserNotesGateway.saveDocument({document: {...a.document, blocks: [{id: noteBlockId("link"), type: "researchObjectReference", props: {}, children: [], content: {kind: "reference", targetKind: "note_page", targetId: b.page.id, labelSnapshot: "leaf", quoteSnapshot: ""}}]}, expectedRevision: 0, saveReason: "structure"});
      client!.notifyChanged();
    });
    const toggle = host.querySelector<HTMLButtonElement>('[aria-label="展开parent中的页面链接"]');
    expect(toggle).not.toBeNull();
    await act(async () => toggle!.click());
    expect(host.querySelector('[aria-label="打开笔记：leaf"]')).not.toBeNull();
    expect(host.querySelector('[aria-label="展开leaf中的页面链接"]')).toBeNull();
    await act(async () => {
      // An inline @ mention is still a leaf in this navigation tree.
      await browserNotesGateway.saveDocument({document: {...a.document, blocks: [{id: noteBlockId("mention"), type: "paragraph", props: {}, children: [], content: {kind: "inline", items: [{type: "mention", targetKind: "note_page", targetId: b.page.id, labelSnapshot: "leaf"}]}}]}, expectedRevision: 1, saveReason: "structure"});
      client!.notifyChanged();
    });
    expect(host.querySelector(".note-favorite-toggle")).toBeNull();
    expect(host.querySelector('[aria-label="打开笔记：leaf"]')).toBeNull();
    expect(host.textContent).not.toContain("暂无页面链接");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
it("expands body block links independently, keeps duplicate links and stops page-reference cycles", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.clear(); selectAppStorage(localStorage);
  const a = await browserNotesGateway.createPage({ title: "a" }), b = await browserNotesGateway.createPage({ title: "b" });
  const block = (id: string, targetId: string) => ({ id: noteBlockId(id), type: "researchObjectReference" as const, props: {}, children: [], content: { kind: "reference" as const, targetKind: "note_page" as const, targetId, labelSnapshot: targetId === a.page.id ? "a" : "b", quoteSnapshot: "" } });
  await browserNotesGateway.saveDocument({ document: { ...a.document, blocks: [block("one", b.page.id), block("two", b.page.id)] }, expectedRevision: 0, saveReason: "structure" });
  await browserNotesGateway.saveDocument({ document: { ...b.document, blocks: [block("back", a.page.id)] }, expectedRevision: 0, saveReason: "structure" });
  const item: NoteIndexItem = { id: `page:${a.page.id}`, title: "a", snippet: "", category: "independent", updatedAt: "", status: "active", isFavorite: true, target: { kind: "page", pageId: a.page.id } };
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<NotesProvider gateway={browserNotesGateway}><NoteFavorites items={[item]} selectedPageId={undefined} onOpen={vi.fn()} onReorder={vi.fn()} onAddInside={vi.fn()} onAction={vi.fn()} disabled={false} /></NotesProvider>));
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="展开a中的页面链接"]')!.click());
    expect(host.querySelectorAll('[aria-label="打开笔记：b"]')).toHaveLength(2);
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="展开b中的页面链接"]')!.click());
    expect(host.querySelector<HTMLButtonElement>('[data-depth="2"] .note-favorite-toggle')!.disabled).toBe(true);
    expect(host.querySelectorAll('[data-depth="1"]')).toHaveLength(2);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
