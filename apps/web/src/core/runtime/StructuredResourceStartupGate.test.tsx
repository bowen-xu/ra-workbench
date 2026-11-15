// @vitest-environment happy-dom
import { act, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AppRuntime } from "./appRuntime";
import type { StructuredResourceGateway } from "../services/structuredResourceGateway";
import { StructuredResourceStartupGate } from "./StructuredResourceStartupGate";
import { ensureStructuredResourceStartup } from "./structuredResourceStartup";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
function storage(): Storage { const data = new Map<string, string>(); return {
  get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
  getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); },
  removeItem: key => { data.delete(key); }, clear: () => data.clear(),
}; }
const summary = {paperIds: ["unopened-paper"], importedPaperCount: 1, importedTemplateCount: 2};

it("does not mount ordinary chat/context clients until the shared startup import finishes", async () => {
  let finish!: (value: typeof summary) => void;
  const importStartup = vi.fn(() => new Promise<typeof summary>(resolve => { finish = resolve; }));
  const runtime: AppRuntime = {mode: "preview", storage: storage(), dispose() {},
    gateways: {getStructuredResourceGateway: {importStartup} as unknown as StructuredResourceGateway}};
  const opened = vi.fn();
  function OrdinaryChat() { useEffect(opened, []); return <div>ordinary chat</div>; }
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<StructuredResourceStartupGate runtime={runtime}><OrdinaryChat/></StructuredResourceStartupGate>));
    expect(opened).not.toHaveBeenCalled();
    expect(host.querySelector('[role="status"]')).not.toBeNull();
    const sameAttempt = ensureStructuredResourceStartup(runtime);
    expect(importStartup).toHaveBeenCalledOnce();
    await act(async () => { finish(summary); await sameAttempt; });
    expect(opened).toHaveBeenCalledOnce();
    expect(host.textContent).toBe("ordinary chat");
  } finally { await act(async () => root.unmount()); }
});

it("keeps a corrupt preference intact, blocks all imports and offers a retry after repair", async () => {
  const saved = storage(), key = "research-assistant.reading-presentation.v1:unopened-paper";
  saved.setItem(key, "{");
  const importStartup = vi.fn(async () => summary), opened = vi.fn();
  const runtime: AppRuntime = {mode: "preview", storage: saved, dispose() {},
    gateways: {getStructuredResourceGateway: {importStartup} as unknown as StructuredResourceGateway}};
  function OrdinaryChat() { useEffect(opened, []); return <div>ready</div>; }
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<StructuredResourceStartupGate runtime={runtime}><OrdinaryChat/></StructuredResourceStartupGate>));
    expect(host.querySelector('[role="alert"]')?.textContent).toContain(key);
    expect(saved.getItem(key)).toBe("{");
    expect(importStartup).not.toHaveBeenCalled();
    expect(opened).not.toHaveBeenCalled();
    saved.setItem(key, JSON.stringify({fields: [], hiddenUnitIds: []}));
    await act(async () => host.querySelector<HTMLButtonElement>("button")!.click());
    expect(importStartup).toHaveBeenCalledOnce();
    expect(opened).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); }
});
