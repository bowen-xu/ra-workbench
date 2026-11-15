// @vitest-environment happy-dom
import { act, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { useVisibleLayoutMeasurement } from "./useVisibleLayoutMeasurement";

let root: ReturnType<typeof createRoot> | undefined;
let host: HTMLDivElement;
const resizes: { run: () => void; disconnect: ReturnType<typeof vi.fn> }[] = [];
let width = 1_200;

afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  root = undefined; host?.remove(); resizes.length = 0;
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});

function setup() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class {
    disconnect = vi.fn();
    constructor(run: () => void) { resizes.push({ run, disconnect: this.disconnect }); }
    observe() {}
  });
  width = 1_200;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    return this.closest("[hidden]") ? new DOMRect() : new DOMRect(0, 0, width, 600);
  });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function (this: HTMLElement) {
    return this.closest("[hidden]") ? 0 : width;
  });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
}

it("keeps the latest callback, rebinds changed DOM owners and disconnects on unmount", async () => {
  setup();
  const measured = vi.fn();
  function Harness({ owner, label }: { readonly owner: string; readonly label: string }) {
    const target = useRef<HTMLDivElement>(null);
    useVisibleLayoutMeasurement(() => target.current, (_element, bounds) => measured(label, bounds.width));
    return <div key={owner} ref={target} />;
  }
  await act(async () => root!.render(<Harness owner="one" label="old" />));
  await act(async () => root!.render(<Harness owner="one" label="new" />));
  expect(resizes).toHaveLength(1);
  measured.mockClear();
  await act(async () => resizes[0]!.run());
  expect(measured).toHaveBeenLastCalledWith("new", 1_200);
  await act(async () => root!.render(<Harness owner="two" label="new" />));
  expect(resizes[0]!.disconnect).toHaveBeenCalledTimes(1);
  expect(resizes).toHaveLength(2);
  await act(async () => root!.unmount()); root = undefined;
  expect(resizes[1]!.disconnect).toHaveBeenCalledTimes(1);
});

it("remeasures committed reveals even without ResizeObserver and does not loop on equal sizes", async () => {
  setup(); vi.stubGlobal("ResizeObserver", undefined);
  const commits = vi.fn();
  function Harness({ hidden }: { readonly hidden: boolean }) {
    const target = useRef<HTMLDivElement>(null), [size, setSize] = useState(0);
    useVisibleLayoutMeasurement(() => target.current, (_element, bounds) => { commits(); setSize(bounds.width); });
    return <div hidden={hidden} ref={target}>{size}</div>;
  }
  await act(async () => root!.render(<Harness hidden={false} />));
  expect(host.textContent).toBe("1200");
  await act(async () => root!.render(<Harness hidden />));
  width = 800;
  await act(async () => root!.render(<Harness hidden={false} />));
  expect(host.textContent).toBe("800");
  expect(commits.mock.calls.length).toBeLessThan(10);
});