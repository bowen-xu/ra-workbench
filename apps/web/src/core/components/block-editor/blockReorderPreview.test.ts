// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { createBlockReorderPreview } from "./blockReorderPreview";

let editor: HTMLElement;
beforeEach(() => {
  document.body.innerHTML = '<div class="block-editor" data-editor-instance="preview"><div class="bn-editor"><div class="bn-block-group"><div class="bn-block-outer" data-id="parent"><div class="bn-block" data-id="parent"><div class="bn-block-content">Parent</div><div class="bn-block-group"><div class="bn-block-outer" data-id="child"><div class="bn-block" data-id="child"><div class="bn-block-content">Child</div></div></div></div></div></div><div class="bn-block-outer" data-id="peer"><div class="bn-block" data-id="peer"><div class="bn-block-content">Peer</div></div></div></div></div></div>';
  editor = document.querySelector<HTMLElement>(".bn-editor")!;
  for (const [id, top, height] of [["parent", 100, 44], ["child", 124, 20], ["peer", 148, 30]] as const) {
    vi.spyOn(editor.querySelector<HTMLElement>(`.bn-block-outer[data-id="${id}"]`)!, "getBoundingClientRect").mockReturnValue(new DOMRect(0, top, 400, height));
  }
});
afterEach(() => { document.querySelectorAll("style[data-block-reorder-preview]").forEach(style => style.remove()); document.body.innerHTML = ""; vi.restoreAllMocks(); });

it("moves the entire selected subtree and its peer without touching editor DOM", () => {
  const before = editor.outerHTML;
  const feedback = createBlockReorderPreview(editor, ["parent"]);
  feedback.update({ blockId: "peer", placement: "after" });
  expect(getComputedStyle(editor.querySelector('[data-id="parent"]')!).transform).toBe("translateY(34px)");
  expect(getComputedStyle(editor.querySelector('[data-id="peer"]')!).transform).toBe("translateY(-48px)");
  expect(getComputedStyle(editor.querySelector('[data-id="child"]')!).transform).toBe("translateY(0px)");
  expect(editor.outerHTML).toBe(before);
  feedback.dispose();
  expect(document.querySelector("style[data-block-reorder-preview]")).toBeNull();
});
it("detaches a child visually without translating it twice with its old parent", () => {
  const feedback = createBlockReorderPreview(editor, ["child"]);
  feedback.update({ blockId: "peer", placement: "after" });
  expect(getComputedStyle(editor.querySelector('[data-id="peer"]')!).transform).toBe("translateY(-20px)");
  expect(getComputedStyle(editor.querySelector('[data-id="child"]')!).transform).toBe("translateY(38px)");
  feedback.update(undefined);
  expect(getComputedStyle(editor.querySelector('[data-id="child"]')!).transform).toBe("");
  feedback.dispose();
});
