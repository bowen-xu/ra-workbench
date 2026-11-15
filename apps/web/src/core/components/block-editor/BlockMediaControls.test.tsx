// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {BlockMediaControls} from "./BlockMediaControls";
import {EditorReadOnlyContext} from "./EditorReadOnly";

let host: HTMLDivElement, root: Root, width: number, resize: (() => void)[];
beforeEach(() => {
  width = 64; resize = [];
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class {
    private readonly callback: () => void;
    constructor(callback: () => void) {this.callback = callback;}
    observe(target: Element) {if (target.classList.contains("block-media-surface")) resize.push(this.callback);}
    unobserve() {} disconnect() {}
  });
  const original = HTMLElement.prototype.getBoundingClientRect;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.classList.contains("block-media-surface") ? {...original.call(this), width} : original.call(this);
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); document.querySelectorAll("[data-return-owner]").forEach(node => node.remove()); vi.restoreAllMocks(); vi.unstubAllGlobals();});
const byLabel = (label: string) => document.querySelector<HTMLElement>(`[aria-label="${label}"]`)!;
const byText = (text: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === text)!;

it.each(["图片", "表格"] as const)("collapses narrow %s controls into the same note menu and expands when resized", async label => {
  const align = vi.fn(), caption = vi.fn();
  await act(async () => root.render(<div className="block-media-surface"><BlockMediaControls label={label} alignment="left"
    onAlignmentChange={align} onCaptionEdit={caption}/></div>));
  expect(byLabel(`${label}操作菜单`)).not.toBeNull();
  expect(host.querySelectorAll("button")).toHaveLength(1);
  await act(async () => byLabel(`${label}操作菜单`).click());
  await vi.waitFor(() => expect(byText("编辑说明")).toBeDefined());
  expect(byText("编辑说明").closest('[data-overlay-portal="popup"]')).not.toBeNull();
  expect(byText("编辑说明").classList.contains("note-action-menu-item")).toBe(true);
  await act(async () => byText("编辑说明").click()); expect(caption).toHaveBeenCalledOnce();
  await vi.waitFor(() => expect(byText("编辑说明")).toBeUndefined());
  await act(async () => byLabel(`${label}操作菜单`).click());
  await vi.waitFor(() => expect(byLabel(`${label}对齐方式`)).not.toBeNull());
  await act(async () => byLabel(`${label}对齐方式`).click());
  await vi.waitFor(() => expect(byText("右对齐")).toBeDefined());
  await act(async () => byText("右对齐").click()); expect(align).toHaveBeenCalledExactlyOnceWith("right");
  await act(async () => {width = 240; resize.forEach(callback => callback());});
  expect(byLabel(`${label}操作菜单`)).toBeNull();
  expect(byLabel(`${label}对齐方式`)).not.toBeNull();
  expect(host.querySelectorAll("button")).toHaveLength(2);
});

it("dismisses a narrow image menu and removes its commands when read-only begins", async () => {
  const changed = vi.fn();
  const content = (readOnly: boolean) => <EditorReadOnlyContext.Provider value={readOnly}><div className="block-media-surface">
    <BlockMediaControls label="图片" alignment="left" onAlignmentChange={changed} onCaptionEdit={changed}/>
  </div></EditorReadOnlyContext.Provider>;
  await act(async () => root.render(content(false)));
  await act(async () => byLabel("图片操作菜单").click());
  await vi.waitFor(() => expect(byText("编辑说明")).toBeDefined());
  await act(async () => root.render(content(true)));
  await vi.waitFor(() => expect(byText("编辑说明")).toBeUndefined());
  expect(host.querySelector("button")).toBeNull(); expect(changed).not.toHaveBeenCalled();
});

it.each([64, 240])("returns alignment focus to the owner at width %i and does not steal caption focus", async size => {
  width = size;
  const owner = document.createElement("button"); owner.dataset.returnOwner = "true"; document.body.append(owner);
  const returnFocus = vi.fn(() => owner.focus()), caption = vi.fn();
  await act(async () => root.render(<div className="block-media-surface"><BlockMediaControls label="图片" alignment="left"
    onAlignmentChange={vi.fn()} onCaptionEdit={caption} onReturnFocus={returnFocus}/></div>));
  if (size === 64) await act(async () => byLabel("图片操作菜单").click());
  await vi.waitFor(() => expect(byLabel("图片对齐方式")).not.toBeNull());
  await act(async () => byLabel("图片对齐方式").click());
  await vi.waitFor(() => expect(size === 64 ? byText("右对齐") : byLabel("右对齐")).not.toBeNull());
  await act(async () => (size === 64 ? byText("右对齐") : byLabel("右对齐")).click());
  await vi.waitFor(() => expect(document.activeElement).toBe(owner));
  expect(returnFocus).toHaveBeenCalledOnce();
  if (size === 64) await act(async () => byLabel("图片操作菜单").click());
  await vi.waitFor(() => expect(size === 64 ? byText("编辑说明") : byLabel("编辑图片说明")).not.toBeNull());
  await act(async () => (size === 64 ? byText("编辑说明") : byLabel("编辑图片说明")).click());
  expect(caption).toHaveBeenCalledOnce(); expect(returnFocus).toHaveBeenCalledOnce();
});

it("returns to the owner when Escape closes the compact menu", async () => {
  const returnFocus = vi.fn();
  await act(async () => root.render(<div className="block-media-surface"><BlockMediaControls label="图片" alignment="left"
    onCaptionEdit={vi.fn()} onReturnFocus={returnFocus}/></div>));
  await act(async () => byLabel("图片操作菜单").click());
  await vi.waitFor(() => expect(byText("编辑说明")).toBeDefined());
  await act(async () => byText("编辑说明").dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true, cancelable: true})));
  await vi.waitFor(() => expect(byText("编辑说明")).toBeUndefined());
  expect(returnFocus).toHaveBeenCalledOnce();
});
