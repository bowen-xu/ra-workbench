// @vitest-environment happy-dom
import { afterEach, describe, expect, it } from "vitest";
import { getEditorScrollContainer, ownsEditorHoverEvent } from "./editorSurface";

afterEach(() => { document.body.innerHTML = ""; });

describe("host-independent editor scrolling", () => {
  it("uses each host's own nearest scroll container without note classes", () => {
    document.body.innerHTML = '<main style="overflow-y:auto"><section style="overflow-y:auto"><div class="bn-editor"></div></section><aside style="overflow-y:scroll"><div class="bn-editor"></div></aside></main>';
    const editors = document.querySelectorAll<HTMLElement>(".bn-editor");
    expect(getEditorScrollContainer(editors[0]!)).toBe(document.querySelector("section"));
    expect(getEditorScrollContainer(editors[1]!)).toBe(document.querySelector("aside"));
  });
  it("honors an explicit host boundary for custom scrolling", () => {
    document.body.innerHTML = '<main data-block-editor-scroll-container><section><div class="bn-editor"></div></section></main>';
    expect(getEditorScrollContainer(document.querySelector<HTMLElement>(".bn-editor")!)).toBe(document.querySelector("main"));
  });
});

describe("editor hover ownership", () => {
  it("admits its whitespace and portal, but rejects unrelated surfaces and other editors", () => {
    document.body.innerHTML = '<main style="overflow-y:auto"><section class="block-editor" data-editor-instance="a"><div class="bn-editor"></div><span id="gutter"></span></section><aside id="panel"><button id="external">Panel</button></aside></main><div data-editor-overlay-owner="a"><button id="own">Handle</button></div><div data-editor-overlay-owner="b"><button id="other">Other handle</button></div>';
    const editor = document.querySelector<HTMLElement>(".bn-editor")!;
    const admitted = (selector: string) => {
      let result: boolean | undefined;
      const observe = (event: Event) => { result = ownsEditorHoverEvent(editor, event); };
      document.addEventListener("mousemove", observe);
      document.querySelector(selector)!.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
      document.removeEventListener("mousemove", observe);
      return result;
    };
    expect(admitted("#gutter")).toBe(true);
    expect(admitted("#own")).toBe(true);
    expect(admitted("main")).toBe(true);
    expect(admitted("#external")).toBe(false);
    expect(admitted("#other")).toBe(false);
    document.querySelector("main")!.insertAdjacentHTML("beforeend", '<section class="block-editor"><div id="other-editor"></div></section>');
    expect(admitted("#other-editor")).toBe(false);
    expect(admitted("main")).toBe(false);
  });
});
