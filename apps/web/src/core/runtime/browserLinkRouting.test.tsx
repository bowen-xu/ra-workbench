// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AppRuntimeProvider } from "./AppRuntimeProvider";
import { AgentMarkdown } from "../features/agent/AgentMarkdown";
import { PaperAiReadingMarkdown } from "../../../../../plugins/literature_library/src/features/reader/PaperAiReadingMarkdown";
import { installBrowserLinkRouting } from "./browserLinkRouting";
import { notify } from "../services/notifications";
import type { AppRuntime } from "./appRuntime";

vi.mock("../services/notifications", () => ({ notify: { error: vi.fn() } }));
const dispose: (() => void)[] = [];
afterEach(() => { dispose.splice(0).reverse().forEach(cleanup => cleanup()); document.body.innerHTML = ""; vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks(); });
function harness(open = vi.fn(async (_url: string) => {})) {
  const runtime: AppRuntime = { mode: "desktop", storage: localStorage, gateways: {}, browser: { open }, dispose() {} };
  return { runtime, open };
}

it("routes real chat and reading Markdown through the application owner and removes listeners on unmount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); dispose.push(() => act(() => root.unmount()));
  const {runtime, open} = harness();
  await act(async () => root.render(<AppRuntimeProvider runtime={runtime}>
    <AgentMarkdown value="[chat](https://arxiv.org/abs/1)" />
    <PaperAiReadingMarkdown value="[reading](http://example.org/paper)" onJump={vi.fn()} />
  </AppRuntimeProvider>));
  await act(async () => { host.querySelectorAll<HTMLAnchorElement>("a").forEach(anchor => anchor.click()); });
  expect(open.mock.calls.map(args => args[0])).toEqual(["https://arxiv.org/abs/1", "http://example.org/paper"]);
  await act(async () => root.render(null));
  const anchor = document.createElement("a"); anchor.href = "https://example.org/"; document.body.append(anchor); anchor.click();
  expect(open).toHaveBeenCalledTimes(2);
});

it("takes ownership before editor window.open, including nested content, modifier and middle clicks", async () => {
  const {runtime, open} = harness(); dispose.push(installBrowserLinkRouting(document, runtime));
  document.body.innerHTML = '<a href="https://example.net/论文"><span>link</span></a>';
  const anchor = document.querySelector("a")!, span = document.querySelector("span")!;
  const editorOpen = vi.fn(); anchor.addEventListener("click", editorOpen);
  for (const init of [{}, {ctrlKey: true}, {metaKey: true}, {shiftKey: true}]) {
    const event = new MouseEvent("click", {bubbles: true, cancelable: true, ...init});
    span.dispatchEvent(event); expect(event.defaultPrevented).toBe(true);
  }
  const middle = new MouseEvent("auxclick", {button: 1, bubbles: true, cancelable: true}); span.dispatchEvent(middle);
  await Promise.resolve();
  expect(editorOpen).not.toHaveBeenCalled(); expect(open).toHaveBeenCalledTimes(5);
  expect(middle.defaultPrevented).toBe(true);
});

it("leaves resources, page destinations, downloads, other protocols and context menus to their current owners", () => {
  const {runtime, open} = harness(); dispose.push(installBrowserLinkRouting(document, runtime));
  for (const [href, download, type, button] of [
    ["source://notes/n", false, "click", 0], ["#page=2", false, "click", 0], ["../local", false, "click", 0],
    ["mailto:a@example.org", false, "click", 0], ["https://example.org", true, "click", 0], ["https://example.org", false, "auxclick", 2],
  ] as const) {
    const anchor = document.createElement("a"); anchor.setAttribute("href", href); if (download) anchor.download = "file"; document.body.append(anchor);
    const owner = vi.fn(); anchor.addEventListener(type, owner);
    const event = new MouseEvent(type, {button, bubbles: true, cancelable: true}); anchor.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false); expect(owner).toHaveBeenCalledOnce();
  }
  expect(open).not.toHaveBeenCalled();
});

it("shows the failure code and native cause instead of an unhandled rejection", async () => {
  const {runtime} = harness(vi.fn(async () => { throw new Error("OS error 5"); }));
  dispose.push(installBrowserLinkRouting(document, runtime));
  const anchor = document.createElement("a"); anchor.href = "https://example.org/"; document.body.append(anchor);
  await act(async () => anchor.click());
  expect(notify.error).toHaveBeenCalledExactlyOnceWith("[BROWSER_OPEN_FAILED] 无法打开网页：OS error 5");
});

it("routes real PDF.js external annotations while retaining internal PDF destination navigation", async () => {
  await import("pdfjs-dist/legacy/build/pdf.mjs");
  const {EventBus} = await import("pdfjs-dist/legacy/web/pdf_viewer.mjs");
  const {ReaderPdfLinkService} = await import("../components/pdf-view/ReaderPdfLinkService");
  const {runtime, open} = harness(); dispose.push(installBrowserLinkRouting(document, runtime));
  const navigate = vi.fn();
  const service = new ReaderPdfLinkService({eventBus: new EventBus()}, navigate);
  const anchor = document.createElement("a");
  service.addLinkAttributes(anchor, "https://doi.org/10.123/example"); document.body.append(anchor);
  anchor.click(); await Promise.resolve();
  expect(open).toHaveBeenCalledExactlyOnceWith("https://doi.org/10.123/example");
  await service.goToDestination("section-2");
  expect(navigate).toHaveBeenCalledExactlyOnceWith({destination: "section-2"});
  expect(open).toHaveBeenCalledTimes(1);
});
