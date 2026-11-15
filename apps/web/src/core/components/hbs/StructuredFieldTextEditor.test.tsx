// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { StructuredFieldTextEditor } from "./StructuredFieldTextEditor";

const cleanup: (() => Promise<void>)[] = [];
afterEach(async () => { for (const dispose of cleanup.splice(0)) await dispose(); vi.unstubAllGlobals(); });
async function setup(initialCaret?: number) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), commit = vi.fn(), change = vi.fn();
  cleanup.push(async () => { await act(async () => root.unmount()); host.remove(); });
  await act(async () => root.render(<StructuredFieldTextEditor label="编辑正文" value="文字🙂和换行\n第二行" disabled={false}
    {...(initialCaret === undefined ? {} : { initialCaret })} onCommit={commit} onChange={change} />));
  return { host, input: host.querySelector("textarea")!, commit, change };
}
it("focuses at the captured UTF-16 character offset without selecting the double-clicked word", async () => {
  const f = await setup(4);
  expect(document.activeElement).toBe(f.input);
  expect(f.input.selectionStart).toBe(4); expect(f.input.selectionEnd).toBe(4);
  expect(f.host.querySelector("button")).toBeNull();
  expect(f.host.querySelector('[aria-hidden="true"]')?.textContent).toBe(f.input.value);
});
it("commits on leaving the input and lets unmodified Enter remain a newline", async () => {
  const f = await setup();
  const enter = new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true });
  await act(async () => { f.input.dispatchEvent(enter); });
  expect(enter.defaultPrevented).toBe(false); expect(f.commit).not.toHaveBeenCalled();
  await act(async () => f.input.blur()); expect(f.commit).toHaveBeenCalledTimes(1);
});
it("Escape commits once even when leaving the input also emits blur", async () => {
  const f = await setup();
  await act(async () => {
    f.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    f.input.blur();
  });
  expect(f.commit).toHaveBeenCalledTimes(1);
});
it("does not save or leave an active IME composition", async () => {
  const f = await setup();
  await act(async () => { f.input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", isComposing: true, bubbles: true })); });
  expect(f.commit).not.toHaveBeenCalled();
});
