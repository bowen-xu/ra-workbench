/** @vitest-environment happy-dom */
import {Fragment, act, useLayoutEffect, useRef} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {useTranscriptWindow} from "./useTranscriptWindow";
import {useNestedScrollRegion, useNestedWheelScroll} from "../../../components/scroll/useNestedWheelScroll";

const items = Array.from({length: 5}, (_, index) => ({id: `row-${index}`, messages: []}));
let heights: Map<string, number>, initialTop: number, viewportHeight: number;
let host: HTMLDivElement, root: Root, frames: Map<number, FrameRequestCallback>;
let positions: WeakMap<Element, number>, writes: number[], frameId: number;

function isPane(node: Element): boolean {return node.classList.contains("test-pane");}
function ensurePosition(node: HTMLElement): void {
  if (positions.has(node)) return;
  positions.set(node, isPane(node) ? initialTop : 0);
  Object.defineProperty(node, "scrollTop", {configurable: true,
    get: () => positions.get(node)!,
    set: (value: number) => {
      const next = Math.max(0, Math.min(Math.max(0, node.scrollHeight - node.clientHeight), value));
      positions.set(node, next); if (isPane(node)) writes.push(next);
    },
  });
}
function height(node: HTMLElement): number {
  return node.dataset.transcriptRow ? heights.get(node.dataset.transcriptRow) ?? 180 : parseFloat(node.style.height) || 0;
}
function bodyHeight(body: Element | null): number {
  return body ? [...body.children].reduce((sum, child) => sum + height(child as HTMLElement), 0) : 0;
}

function NestedInput() {
  const ref = useRef<HTMLDivElement>(null); useNestedScrollRegion(ref);
  return <div ref={ref} data-testid="inner" />;
}
function Window({rows = items, estimate = 180}: {readonly rows?: typeof items; readonly estimate?: number}) {
  const windowed = useTranscriptWindow(rows, undefined, {fromEnd: false, estimate});
  return <div ref={windowed.body} data-window-body>
    {windowed.rows.map(({item, gap, index}) => <Fragment key={item.id}>
      {gap > 0 && <div aria-hidden="true" style={{height: gap}} />}
      <div data-transcript-row={item.id}>{item.id}{index === 0 && <NestedInput />}</div>
    </Fragment>)}
    {windowed.bottom > 0 && <div aria-hidden="true" style={{height: windowed.bottom}} />}
  </div>;
}
function Fixture({rows = items, onCommit, follow = false, estimate = 180}: {
  readonly rows?: typeof items; readonly onCommit?: (() => void) | undefined;
  readonly follow?: boolean; readonly estimate?: number;
}) {
  const pane = useRef<HTMLDivElement>(null); useNestedWheelScroll(pane);
  useLayoutEffect(() => {onCommit?.();}, [onCommit]);
  return <div ref={pane} className="test-pane writing-agent-messages" data-follow-bottom={String(follow)}>
    <Window rows={rows} estimate={estimate} />
  </div>;
}

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  heights = new Map([["row-0", 800]]); initialTop = 500; viewportHeight = 200;
  positions = new WeakMap(); writes = []; frames = new Map(); frameId = 0;
  const schedule = (callback: FrameRequestCallback) => {frames.set(++frameId, callback); return frameId;};
  const cancel = (id: number) => {frames.delete(id);};
  vi.stubGlobal("requestAnimationFrame", schedule); vi.stubGlobal("cancelAnimationFrame", cancel);
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(schedule);
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(cancel);
  vi.stubGlobal("ResizeObserver", class {observe() {} disconnect() {}});
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function(this: HTMLElement) {
    if (isPane(this)) {ensurePosition(this); return viewportHeight;}
    if (this.dataset.testid === "inner") {ensurePosition(this); return 100;}
    return 0;
  });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(600);
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(function(this: HTMLElement) {
    if (isPane(this)) return bodyHeight(this.querySelector("[data-window-body]"));
    return this.dataset.testid === "inner" ? 1000 : 0;
  });
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    if (isPane(this)) return new DOMRect(0, 100, 600, viewportHeight);
    const pane = this.closest<HTMLElement>(".test-pane");
    const body = pane?.querySelector<HTMLElement>("[data-window-body]");
    if (!pane || !body) return new DOMRect();
    ensurePosition(pane);
    const top = 100 - pane.scrollTop;
    if (this === body) return new DOMRect(0, top, 600, bodyHeight(body));
    if (this.parentElement === body) {
      let offset = 0;
      for (const child of body.children) {if (child === this) break; offset += height(child as HTMLElement);}
      return new DOMRect(0, top + offset, 600, height(this));
    }
    return new DOMRect(0, top, 600, 100);
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function flushFrames(): Promise<void> {
  for (let i = 0; i < 16 && frames.size; i++) {
    const callbacks = [...frames.values()]; frames.clear();
    await act(async () => callbacks.forEach(callback => callback(i * 16)));
  }
  expect(frames.size).toBe(0);
}
const pane = () => host.querySelector<HTMLElement>(".test-pane")!;

it("anchors the actually visible expanded row instead of a row guessed from old estimates", async () => {
  await act(async () => root.render(<Fixture />)); await flushFrames();
  expect(pane().scrollTop).toBe(500);
  expect(writes.every(value => value === 500)).toBe(true);
});

it("keeps the outer position when an inner wheel gesture arrives between capture and restoration", async () => {
  const onCommit = () => {
    const inner = host.querySelector<HTMLElement>('[data-testid="inner"]')!;
    // Production wheel routing stops propagation at the outer pane. The pending
    // outer anchor must still be invalidated before the inner delta is applied.
    inner.dispatchEvent(new WheelEvent("wheel", {bubbles: true, cancelable: true, deltaY: 20}));
  };
  await act(async () => root.render(<Fixture onCommit={onCommit} />)); await flushFrames();
  expect(pane().scrollTop).toBe(500);
  expect(host.querySelector<HTMLElement>('[data-testid="inner"]')!.scrollTop).toBe(20);
});

it.each(["wheel", "pointerdown", "touchstart", "keydown"])("does not restore a stale snapshot after new %s input", async type => {
  const onCommit = () => {
    pane().dispatchEvent(type === "wheel"
      ? new WheelEvent(type, {bubbles: true, cancelable: true, deltaY: 0})
      : new Event(type, {bubbles: true}));
    pane().scrollTop = 650;
  };
  await act(async () => root.render(<Fixture onCommit={onCommit} />)); await flushFrames();
  expect(pane().scrollTop).toBe(650);
});

it("compensates growth above the visible row while preserving that row's screen position", async () => {
  initialTop = 850;
  await act(async () => root.render(<Fixture />)); await flushFrames();
  const before = host.querySelector<HTMLElement>('[data-transcript-row="row-1"]')!.getBoundingClientRect().top;
  heights.set("row-0", 1000);
  await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
  expect(pane().scrollTop).toBe(1050);
  expect(host.querySelector<HTMLElement>('[data-transcript-row="row-1"]')!.getBoundingClientRect().top).toBe(before);
});

it.each([false, true])("honors explicit bottom-follow intent across height changes (follow=%s)", async follow => {
  initialTop = 780; viewportHeight = 200;
  heights = new Map([["row-0", 800], ["row-1", 180]]);
  const rows = items.slice(0, 2);
  await act(async () => root.render(<Fixture rows={rows} follow={follow} />)); await flushFrames();
  expect(pane().scrollTop).toBe(780);
  heights.set("row-0", 1000);
  await act(async () => root.render(<Fixture rows={[...rows]} follow={follow} />)); await flushFrames();
  expect(pane().scrollTop).toBe(follow ? 980 : 780);
});

it("also preserves a tall tool-history row when its 24px estimates disagree with actual geometry", async () => {
  const rows = Array.from({length: 40}, (_, index) => ({id: `row-${index}`, messages: []}));
  await act(async () => root.render(<Fixture rows={rows} estimate={24} />)); await flushFrames();
  expect(pane().scrollTop).toBe(500);
  expect(host.querySelector('[data-transcript-row="row-0"]')).not.toBeNull();
});

it("does not map a deleted anchor to an unrelated row at the same index", async () => {
  await act(async () => root.render(<Fixture />)); await flushFrames();
  writes.length = 0;
  await act(async () => root.render(<Fixture rows={items.slice(1)} />)); await flushFrames();
  expect(writes).toEqual([]);
});

it("discards a snapshot when an external scroll moves the pane without a wheel event", async () => {
  await act(async () => root.render(<Fixture />)); await flushFrames();
  pane().scrollTop = 900;
  await act(async () => pane().dispatchEvent(new Event("scroll"))); await flushFrames();
  const before = host.querySelector<HTMLElement>('[data-transcript-row="row-1"]')!.getBoundingClientRect().top;
  heights.set("row-0", 1000);
  await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
  expect(pane().scrollTop).toBe(1100);
  expect(host.querySelector<HTMLElement>('[data-transcript-row="row-1"]')!.getBoundingClientRect().top).toBe(before);
});

it("ignores input in sibling panes when preserving the visible message", async () => {
  initialTop = 850;
  const sibling = document.createElement("div"); document.body.append(sibling);
  try {
    await act(async () => root.render(<Fixture />)); await flushFrames();
    sibling.dispatchEvent(new WheelEvent("wheel", {bubbles: true, deltaY: 20}));
    heights.set("row-0", 1000);
    await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
    expect(pane().scrollTop).toBe(1050);
  } finally {sibling.remove();}
});

it("does not turn a nonoverflowing history into automatic bottom-following", async () => {
  initialTop = 0; viewportHeight = 1000; heights = new Map();
  await act(async () => root.render(<Fixture />)); await flushFrames();
  heights.set("row-0", 800);
  await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
  expect(pane().scrollTop).toBe(0);
});

it("retains the last measurable anchor while hidden and compensates on reopening", async () => {
  initialTop = 850;
  await act(async () => root.render(<Fixture />)); await flushFrames();
  viewportHeight = 0; heights.set("row-0", 1000); writes.length = 0;
  await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
  expect(writes).toEqual([]);
  viewportHeight = 200;
  await act(async () => root.render(<Fixture rows={[...items]} />)); await flushFrames();
  expect(pane().scrollTop).toBe(1050);
});
