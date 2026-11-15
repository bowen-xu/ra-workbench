// @vitest-environment happy-dom
import {act, useState} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {EditorReadOnlyContext} from "./EditorReadOnly";
import {ImageBlockPresentation, type ImageAlignment} from "./ImageBlockPresentation";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();});
async function press(input: HTMLElement, key: string, isComposing = false): Promise<void> {
  await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key, isComposing, bubbles: true, cancelable: true})));
}
function button(label: string, container: ParentNode = host): HTMLButtonElement {
  const result = container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!result) throw new Error(`按钮未找到：${label}`);
  return result;
}

it("shows only the image with an initially absent empty caption and icon controls", async () => {
  await act(async () => root.render(<ImageBlockPresentation src="blob:image" alt="截图.png" assetId="image-asset"
    caption="" onAlignmentChange={vi.fn()} />));
  const figure = host.querySelector("figure.note-image-block");
  expect(figure).not.toBeNull();
  expect(figure?.querySelector("img")?.getAttribute("src")).toBe("blob:image");
  expect(figure?.querySelector("img")?.getAttribute("alt")).toBe("截图.png");
  expect(figure?.textContent).not.toContain("截图.png");
  expect(figure?.querySelector("figcaption, .block-caption, textarea")).toBeNull();
  const toolbar = figure?.querySelector('[aria-label="图片操作"]');
  expect(toolbar?.querySelectorAll("button")).toHaveLength(1);
  expect(toolbar?.textContent?.trim()).toBe("");
  expect(host.querySelector('button[aria-label="编辑图片说明"]')).toBeNull();
  expect(button("图片对齐方式")).not.toBeNull();
});

it("uses a static caption when rendered outside an editor, without creating a private editable field", async () => {
  await act(async () => root.render(<ImageBlockPresentation src="blob:image" alt="示意图" caption="图 1：算法流程"/>));
  expect(host.querySelector(".block-caption")?.textContent).toBe("图 1：算法流程");
  expect(host.querySelector('[role="textbox"],textarea')).toBeNull();
});

it("uses a shared popup containing only the three alignment icons and returns focus after choosing", async () => {
  const changed = vi.fn();
  function Fixture() {
    const [alignment, setAlignment] = useState<ImageAlignment>("center");
    return <ImageBlockPresentation src="blob:image" alt="示意图" caption="" alignment={alignment}
      onAlignmentChange={next => {changed(next); setAlignment(next);}} />;
  }
  await act(async () => root.render(<Fixture />));
  const trigger = button("图片对齐方式");
  await act(async () => trigger.click());
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="右对齐"]')).not.toBeNull());
  const right = button("右对齐", document), center = button("居中对齐", document), left = button("左对齐", document);
  const portal = right.closest('[data-overlay-portal="popup"]');
  expect(portal).not.toBeNull(); expect(host.contains(right)).toBe(false);
  expect(portal?.querySelectorAll("button")).toHaveLength(3);
  expect(portal?.textContent?.trim()).toBe("");
  expect([left, center, right].map(item => item.getAttribute("aria-pressed"))).toEqual(["false", "true", "false"]);
  await act(async () => right.click());
  expect(changed).toHaveBeenCalledExactlyOnceWith("right");
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="右对齐"]')).toBeNull());
  await vi.waitFor(() => expect(document.activeElement).toBe(trigger));
  await act(async () => trigger.click());
  await vi.waitFor(() => expect(button("右对齐", document).getAttribute("aria-pressed")).toBe("true"));
});

it("defaults to left alignment and dismisses the popup with Escape without modifying the image", async () => {
  const changed = vi.fn();
  await act(async () => root.render(<ImageBlockPresentation src="blob:image" alt="示意图" caption="" onAlignmentChange={changed} />));
  await act(async () => button("图片对齐方式").click());
  await vi.waitFor(() => expect(button("左对齐", document).getAttribute("aria-pressed")).toBe("true"));
  await act(async () => button("左对齐", document).focus());
  await press(button("左对齐", document), "Escape");
  // A focused icon's hint is a child overlay and dismisses before its popup.
  if (document.querySelector('button[aria-label="左对齐"]')) {
    await vi.waitFor(() => expect(document.querySelector('[data-overlay-portal="hint"] .control-hint')).toBeNull());
    await press(button("左对齐", document), "Escape");
  }
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="左对齐"]')).toBeNull());
  expect(changed).not.toHaveBeenCalled();
});

it("renders read-only captions without editable controls and omits an empty read-only caption", async () => {
  const caption = vi.fn(), alignment = vi.fn();
  const render = (value: string) => <EditorReadOnlyContext.Provider value><ImageBlockPresentation src="blob:image"
    alt="示意图" caption={value} onAlignmentChange={alignment} /></EditorReadOnlyContext.Provider>;
  await act(async () => root.render(render("图 1：结果")));
  expect(host.querySelector(".block-caption")?.textContent).toBe("图 1：结果");
  expect(host.querySelector('[aria-label="图片操作"], button, textarea')).toBeNull();
  await act(async () => root.render(render("")));
  expect(host.querySelector("figcaption, .block-caption")).toBeNull();
  expect(caption).not.toHaveBeenCalled(); expect(alignment).not.toHaveBeenCalled();
});

it("closes an open editing popup when the image becomes read-only", async () => {
  const changed = vi.fn();
  const render = (readOnly: boolean) => <EditorReadOnlyContext.Provider value={readOnly}><ImageBlockPresentation
    src="blob:image" alt="示意图" caption="" onAlignmentChange={changed} /></EditorReadOnlyContext.Provider>;
  await act(async () => root.render(render(false)));
  await act(async () => button("图片对齐方式").click());
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="左对齐"]')).not.toBeNull());
  await act(async () => root.render(render(true)));
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="左对齐"]')).toBeNull());
  expect(host.querySelector('[aria-label="图片操作"]')).toBeNull(); expect(changed).not.toHaveBeenCalled();
});

it("preserves actionable image errors and forwards decoding failures", async () => {
  const failed = vi.fn();
  await act(async () => root.render(<ImageBlockPresentation src="blob:image" alt="示意图" caption="" onError={failed} />));
  await act(async () => host.querySelector("img")?.dispatchEvent(new Event("error")));
  expect(failed).toHaveBeenCalledOnce();
  await act(async () => root.render(<ImageBlockPresentation alt="示意图" caption="" error="note_asset_read_failed: 附件读取失败" />));
  expect(host.querySelector('[role="alert"]')?.textContent).toContain("note_asset_read_failed");
  expect(host.querySelector("img")).toBeNull();
});
