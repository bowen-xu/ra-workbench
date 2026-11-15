// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { acceptCompletion, autocompletion, currentCompletions, selectedCompletionIndex,
  setSelectedCompletion, startCompletion, type Completion } from "@codemirror/autocomplete";
import { EditorState } from "@codemirror/state";
import { EditorView, getTooltip, showTooltip } from "@codemirror/view";
import { virtualCompletion } from "./virtualCompletion";

const views: EditorView[] = [];
beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function (this: HTMLElement) {
    return this.tagName === "LI" ? 20 : 140;
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(140);
});
afterEach(() => {
  for (const view of views.splice(0)) view.destroy();
  document.body.replaceChildren(); vi.restoreAllMocks();
});

async function open(options: readonly Completion[] = Array.from({ length: 1000 }, (_, i) => ({ label: `alpha${String(i).padStart(4, "0")}` })), virtual = true) {
  const config = { interactionDelay: 0, override: [() => ({ from: 1, options, validFor: /^[a-z]*$/u })] };
  const view = new EditorView({ parent: document.body, state: EditorState.create({ doc: "\\", selection: { anchor: 1 },
    extensions: [virtual ? virtualCompletion(config) : autocompletion(config)] }) });
  views.push(view); startCompletion(view);
  await vi.waitFor(() => expect(currentCompletions(view.state)).toHaveLength(options.length));
  await vi.waitFor(() => expect(document.querySelector('[role="listbox"]')).not.toBeNull());
  return { view, list: document.querySelector<HTMLElement>('[role="listbox"]')! };
}

it("retains all candidates with only viewport rows in the DOM", async () => {
  const { view, list } = await open();
  expect(currentCompletions(view.state)).toHaveLength(1000);
  expect(list.querySelectorAll('[role="option"]').length).toBeLessThan(20);
  expect(list.querySelector('[role="option"]')?.getAttribute("aria-setsize")).toBe("1000");
});

it("uses SVG type icons without the native Unicode pseudo-element classes", async () => {
  const { list } = await open([{ label: "alpha", type: "keyword" }]);
  const icon = list.querySelector(".cm-completionIcon")!;
  expect(icon.querySelector("svg path")).not.toBeNull();
  expect(icon.getAttribute("aria-hidden")).toBe("true");
  expect(icon.classList.contains("cm-completionIcon-keyword")).toBe(false);
  expect(icon.textContent).toBe("");
});

it("reuses connected list and row nodes when a prefix filters the candidates", async () => {
  const { view, list } = await open();
  const rows = new Set(list.querySelectorAll('[role="option"]'));
  const mutations: MutationRecord[] = [];
  const observer = new MutationObserver(records => mutations.push(...records));
  observer.observe(list, { childList: true });
  view.dispatch({ changes: { from: 1, insert: "a" }, selection: { anchor: 2 }, userEvent: "input.type" });
  await vi.waitFor(() => expect(currentCompletions(view.state)).toHaveLength(1000));
  expect(document.querySelector('[role="listbox"]')).toBe(list);
  for (const row of list.querySelectorAll('[role="option"]')) expect(rows.has(row)).toBe(true);
  observer.disconnect();
  expect(mutations.flatMap(record => [...record.removedNodes])).toEqual([]);
});

it("scrolls continuously to the end without changing selection or losing the active descendant", async () => {
  const { view, list } = await open();
  list.scrollTop = 19_860;
  list.dispatchEvent(new Event("scroll"));
  await vi.waitFor(() => expect(list.querySelector('[aria-posinset="1000"]')).not.toBeNull());
  expect(selectedCompletionIndex(view.state)).toBe(0);
  expect(document.getElementById(view.contentDOM.getAttribute("aria-activedescendant")!)).not.toBeNull();
  expect(list.querySelectorAll('[role="option"]').length).toBeLessThan(20);
  const last = list.querySelector<HTMLElement>('[aria-posinset="1000"]')!;
  const positions = Array.from(list.querySelectorAll('[role="option"]'), row => Number(row.getAttribute("aria-posinset")));
  expect(positions).toEqual([...positions].sort((a, b) => a - b));
  last.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
  expect(view.state.doc.toString()).toBe("\\alpha0999");
});

it("reveals and accepts a keyboard-selected candidate outside the viewport", async () => {
  const { view, list } = await open();
  view.dispatch({ effects: setSelectedCompletion(999) });
  expect(list.scrollTop).toBeGreaterThan(19_000);
  expect(list.querySelector('[aria-selected="true"]')?.textContent).toContain("alpha0999");
  expect(acceptCompletion(view)).toBe(true);
  expect(view.state.doc.toString()).toBe("\\alpha0999");
});

it("does not read scroll geometry during a keyboard selection transaction", async () => {
  const { view, list } = await open();
  const scrollRead = vi.spyOn(list, "scrollTop", "get");
  view.dispatch({ effects: setSelectedCompletion(3) });
  expect(scrollRead).not.toHaveBeenCalled();
});

it("preserves unchanged option content templates when only match highlights change", async () => {
  const { view, list } = await open([{ label: "alpha", type: "function", detail: "symbol" }]);
  const icon = list.querySelector(".cm-completionIcon");
  const detail = list.querySelector(".cm-completionDetail");
  view.dispatch({ changes: { from: 1, insert: "a" }, selection: { anchor: 2 }, userEvent: "input.type" });
  expect(list.querySelector(".cm-completionIcon")).toBe(icon);
  expect(list.querySelector(".cm-completionDetail")).toBe(detail);
  expect(list.querySelector(".cm-completionMatchedText")?.textContent).toBe("a");
});

it("does not mutate stable rows after a redundant tooltip measurement", async () => {
  const { view, list } = await open();
  const descriptor = view.state.facet(showTooltip).find(value => value !== null)!;
  const tooltip = getTooltip(view, descriptor)!;
  const size = { top: 0, left: 0, bottom: 720, right: 1280 };
  tooltip.positioned?.(size);
  const reads = vi.spyOn(list, "clientHeight", "get").mockReturnValue(140);
  await vi.waitFor(() => expect(reads).toHaveBeenCalled());
  const records: MutationRecord[] = [];
  const observer = new MutationObserver(batch => records.push(...batch));
  observer.observe(list, { subtree: true, attributes: true, childList: true });
  reads.mockClear(); tooltip.positioned?.(size);
  await vi.waitFor(() => expect(reads).toHaveBeenCalled());
  observer.disconnect();
  expect(records).toHaveLength(0);
});

it("keeps the upstream layout for section headers and consumers that do not opt in", async () => {
  const options = Array.from({ length: 30 }, (_, i) => ({ label: `option${i}`, section: "Commands" }));
  const { list } = await open(options);
  expect(list.querySelector("completion-section")?.textContent).toBe("Commands");
  expect(list.querySelectorAll('[role="option"]')).toHaveLength(30);
  views.pop()!.destroy(); document.body.replaceChildren();
  const plain = await open(undefined, false);
  expect(plain.list.querySelectorAll('[role="option"]')).toHaveLength(100);
});
