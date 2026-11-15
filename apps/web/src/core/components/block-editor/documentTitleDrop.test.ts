// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {resolveBlockDropTarget, captureBlockDropGeometry} from "./blockDropTarget";

afterEach(() => {vi.restoreAllMocks(); document.body.innerHTML = "";});
it.each([false, true])("resolves the title and whole title/body gap to the first body boundary (captured=%s)", captured => {
  document.body.innerHTML = '<div class="block-editor"><div class="bn-editor"><div class="bn-block-group"><div class="bn-block-outer"><div class="bn-block" data-id="title"><div class="bn-block-content" data-content-type="documentTitle"><h1>Title</h1></div></div></div><div class="bn-block-outer"><div class="bn-block" data-id="body"><div class="bn-block-content"><p>Body</p></div></div></div></div></div></div><aside>Other pane</aside>';
  const editor = document.querySelector<HTMLElement>(".bn-editor")!;
  const title = editor.querySelector<HTMLElement>("[data-content-type=documentTitle]")!, body = editor.querySelector<HTMLElement>("p")!.parentElement!;
  vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 60, 500, 500));
  for (const node of [title, title.closest<HTMLElement>(".bn-block-outer")!]) vi.spyOn(node, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 60, 500, 48));
  for (const node of [body, body.closest<HTMLElement>(".bn-block-outer")!]) vi.spyOn(node, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 160, 500, 30));
  let hit: Element = title.querySelector("h1")!;
  vi.spyOn(document, "elementFromPoint").mockImplementation(() => hit);
  const geometry = captured ? captureBlockDropGeometry(editor) : undefined;
  if (geometry) expect(geometry.rows.some(row => row === title)).toBe(true);
  for (const y of [60, 85, 109, 125, 145, 159, 160]) {
    hit = y < 108 ? title.querySelector("h1")! : editor;
    expect(resolveBlockDropTarget(editor, {clientX: 350, clientY: y}, geometry)).toEqual({
      blockId: "body", placement: "before", line: {left: 100, top: 160, width: 500},
    });
  }
  hit = document.querySelector("aside")!;
  expect(resolveBlockDropTarget(editor, {clientX: 350, clientY: 145}, geometry)).toBeUndefined();
});
