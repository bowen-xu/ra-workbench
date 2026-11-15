// @vitest-environment happy-dom
import {act, useRef} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {useComposerAutosize} from "./useComposerAutosize";
import type {AgentComposerInputHandle} from "./AgentComposerTextInput";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
afterEach(() => {vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.replaceChildren();});
it("grows and shrinks to content, reserves controls inside the 35% cap, and fades only scrolled text", async () => {
  let panelHeight = 500;
  const measuredLiveHeights: string[] = [];
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    const height = this.classList.contains("writing-agent-panel") ? panelHeight
      : this.tagName === "FORM" ? 80 + Number(this.querySelector("textarea")?.style.height.replace("px", "") || 54)
      : Number(this.style.height.replace("px", "") || 54);
    return {height, width: 400, top: 0, left: 0, bottom: height, right: 400, x: 0, y: 0, toJSON: () => ({})};
  });
  vi.spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get").mockImplementation(function(this: HTMLTextAreaElement) {
    const live = document.querySelector<HTMLTextAreaElement>('textarea:not([aria-hidden="true"])');
    if (live) measuredLiveHeights.push(live.style.height);
    return Math.max(54, this.value.split("\n").length * 22 + 8);
  });
  function Harness({text}: {text: string}) {
    const input = useRef<AgentComposerInputHandle>(null);
    useComposerAutosize(input, false, text, 2);
    return <div className="writing-agent-panel"><form className="agent-reference-anchor"><textarea value={text} readOnly ref={element => {input.current = element;}}/></form></div>;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const render = (text: string) => act(async () => root.render(<Harness text={text}/>));
  try {
    await render("一行"); const input = host.querySelector("textarea")!;
    expect(input.style.height).toBe("54px"); expect(input.style.overflowY).toBe("hidden");
    await render("一\n二\n三"); expect(input.style.height).toBe("74px"); expect(input.style.overflowY).toBe("hidden");
    await render(Array(20).fill("行").join("\n"));
    expect(input.style.height).toBe("95px"); expect(input.style.overflowY).toBe("auto");
    input.scrollTop = 20; input.dispatchEvent(new Event("scroll")); expect(input.hasAttribute("data-scrolled-top")).toBe(true);
    input.scrollTop = 0; input.dispatchEvent(new Event("scroll")); expect(input.hasAttribute("data-scrolled-top")).toBe(false);
    panelHeight = 700; await render(Array(21).fill("行").join("\n")); expect(input.style.height).toBe("164px");
    await render("一行"); expect(input.style.height).toBe("54px"); expect(input.style.overflowY).toBe("hidden");
    expect(measuredLiveHeights).not.toContain("0px");
    expect(host.querySelectorAll("textarea")).toHaveLength(1);
  } finally {await act(async () => root.unmount());}
});

it("keeps size observers attached while typing rather than recreating them per character", async () => {
  const observers: { disconnect: ReturnType<typeof vi.fn> }[] = [];
  vi.stubGlobal("ResizeObserver", class {
    observe = vi.fn(); disconnect = vi.fn();
    constructor() { observers.push(this); }
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    const height = this.classList.contains("writing-agent-panel") ? 500 : this.tagName === "FORM" ? 134 : 54;
    return {height, width:400, top:0, left:0, bottom:height, right:400, x:0, y:0, toJSON:()=>({})};
  });
  vi.spyOn(HTMLTextAreaElement.prototype, "scrollHeight", "get").mockReturnValue(54);
  function Harness({text}: {text:string}) {
    const input = useRef<AgentComposerInputHandle>(null);
    useComposerAutosize(input, false, text, 0);
    return <div className="writing-agent-panel"><form className="agent-reference-anchor"><textarea value={text} readOnly ref={node=>{input.current=node;}}/></form></div>;
  }
  const host=document.createElement("div"); document.body.append(host); const root=createRoot(host);
  try {
    for (const text of ["a", "ab", "abc"]) await act(async()=>root.render(<Harness text={text}/>));
    expect(observers).toHaveLength(1);
    expect(observers[0]!.disconnect).not.toHaveBeenCalled();
  } finally {await act(async()=>root.unmount());}
  expect(observers[0]!.disconnect).toHaveBeenCalledOnce();
});
