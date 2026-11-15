// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NoteFloatingWindow, type NoteWindowSession } from "./NoteFloatingWindow";

let root: ReturnType<typeof createRoot> | undefined;
beforeEach(() => vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true));
afterEach(async () => { if (root) await act(async () => root!.unmount()); root = undefined; document.body.replaceChildren(); vi.unstubAllGlobals(); });
const button = (label: string) => document.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
async function setup(title = "阅读笔记") {
  const opener = document.createElement("button"); document.body.append(opener); opener.focus();
  const session = { prepareClose: vi.fn(async () => true), openTab: vi.fn<NoteWindowSession["openTab"]>(async () => true),
    dismiss: vi.fn(), reportError: vi.fn(), beginOperation: vi.fn(() => true), endOperation: vi.fn() };
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const render = async (identity = "one") => act(async () => root!.render(<NoteFloatingWindow identity={identity} title={title} label={`${title}编辑面板`}
    closeLabel={`关闭${title}`} resizeLabel={title} session={session}>{({busy}) => <textarea aria-label="正文" readOnly={busy} defaultValue="preserved" />}</NoteFloatingWindow>));
  await render(); return {session, opener, render};
}

it.each(["阅读笔记", "写作笔记"])("uses the same production window and save/close behavior for %s", async title => {
  const f = await setup(title);
  expect(document.querySelector('.note-floating-window[data-overlay-scope="panel"]')).not.toBeNull();
  expect(document.querySelectorAll('.reading-note-resize')).toHaveLength(4);
  const editor = document.querySelector("textarea");
  f.session.prepareClose.mockResolvedValueOnce(false);
  await act(async () => button(`关闭${title}`).click());
  expect(f.session.dismiss).not.toHaveBeenCalled(); expect(document.querySelector("textarea")).toBe(editor);
  await act(async () => button(`关闭${title}`).click());
  expect(f.session.dismiss).toHaveBeenCalledOnce(); expect(document.activeElement).toBe(f.opener);
});

it("serializes save and expansion, locks input, and dismisses only once after navigation accepts", async () => {
  const f = await setup(); let saved!: (ready: boolean) => void, accept!: () => void, navigated!: () => void;
  f.session.prepareClose.mockImplementationOnce(() => new Promise(resolve => { saved = resolve; }));
  f.session.openTab.mockImplementationOnce(accepted => new Promise<void>(resolve => { accept = accepted; navigated = resolve; }));
  await act(async () => { button("在笔记标签页中打开").click(); button("在笔记标签页中打开").click(); });
  expect(f.session.prepareClose).toHaveBeenCalledOnce(); expect(f.session.openTab).not.toHaveBeenCalled();
  expect(document.querySelector('.note-floating-window-body')?.hasAttribute('inert')).toBe(true);
  expect(document.querySelector('textarea')?.readOnly).toBe(true);
  await act(async () => saved(true));
  expect(f.session.openTab).toHaveBeenCalledOnce(); expect(f.session.dismiss).not.toHaveBeenCalled();
  await act(async () => { accept(); navigated(); });
  expect(f.session.dismiss).toHaveBeenCalledOnce(); expect(f.session.endOperation).toHaveBeenCalledOnce();
});

it("retains content and the original error when expansion fails", async () => {
  const f = await setup(), error = new Error("file_revision_conflict: disk changed");
  f.session.openTab.mockRejectedValueOnce(error);
  await act(async () => button("在笔记标签页中打开").click());
  expect(f.session.dismiss).not.toHaveBeenCalled(); expect(f.session.reportError).toHaveBeenCalledWith(error);
  expect(document.querySelector('textarea')?.value).toBe("preserved");
  expect(document.querySelector('.note-floating-window-body')?.hasAttribute('inert')).toBe(false);
  expect(button("在笔记标签页中打开").disabled).toBe(false);
});

it("leaves nested menus, IME and handled Escape to their owners before closing", async () => {
  const f = await setup(), editor = document.querySelector('textarea')!;
  await act(async () => {
    editor.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape", bubbles:true, isComposing:true}));
    editor.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape", bubbles:true, keyCode:229}));
    const handled = new KeyboardEvent("keydown", {key:"Escape", bubbles:true, cancelable:true}); handled.preventDefault(); editor.dispatchEvent(handled);
    const menu = document.createElement('div'); menu.setAttribute('role', 'menu'); editor.parentElement!.append(menu);
    menu.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape", bubbles:true})); menu.remove();
  });
  expect(f.session.prepareClose).not.toHaveBeenCalled();
  await act(async () => editor.dispatchEvent(new KeyboardEvent("keydown", {key:"Escape", bubbles:true, cancelable:true})));
  expect(f.session.dismiss).toHaveBeenCalledOnce();
});

it("does not deliver a late save/acceptance to a replaced or unmounted document", async () => {
  const f = await setup(); let saved!: (ready: boolean) => void;
  f.session.prepareClose.mockImplementationOnce(() => new Promise(resolve => { saved = resolve; }));
  await act(async () => button("在笔记标签页中打开").click());
  await f.render("two"); await act(async () => saved(true));
  expect(f.session.openTab).not.toHaveBeenCalled(); expect(f.session.dismiss).not.toHaveBeenCalled();
  f.session.prepareClose.mockImplementationOnce(() => new Promise(resolve => { saved = resolve; }));
  await act(async () => button("在笔记标签页中打开").click());
  await act(async () => root!.unmount()); root = undefined; await act(async () => saved(true));
  expect(f.session.dismiss).not.toHaveBeenCalled();
});
