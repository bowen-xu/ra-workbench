// @vitest-environment happy-dom
import {act, useEffect} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import type {StructuredStartupImportResult} from "@research-assistant/core/structured-resources";
import type {AppRuntime} from "./appRuntime";
import type {StructuredResourceGateway} from "../services/structuredResourceGateway";
import {ensureStructuredResourceStartup} from "./structuredResourceStartup";
import {StructuredResourceStartupGate} from "./StructuredResourceStartupGate";

const complete: StructuredStartupImportResult = {paperIds: ["paper"], importedPaperCount: 1, importedTemplateCount: 0};
const deferred: StructuredStartupImportResult = {...complete, paperIds: [], importedPaperCount: 0, deferredPaperIds: ["paper"]};
function pendingResult<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((accept, fail) => {resolve = accept; reject = fail;});
  return {promise, resolve, reject};
}
function runtime(importStartup: StructuredResourceGateway["importStartup"]): AppRuntime {
  const values = new Map<string, string>();
  const storage: Storage = {get length() {return values.size;}, key: i => [...values.keys()][i] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  return {mode: "preview", storage, gateways: {getStructuredResourceGateway: {importStartup} as StructuredResourceGateway}, dispose() {}};
}

it("joins an in-flight deferred import and retries only on the next existing ensure, then caches complete success", async () => {
  const first = pendingResult<StructuredStartupImportResult>(), next = pendingResult<StructuredStartupImportResult>();
  const importStartup = vi.fn<StructuredResourceGateway["importStartup"]>().mockReturnValueOnce(first.promise).mockReturnValueOnce(next.promise);
  const owner = runtime(importStartup), pending = ensureStructuredResourceStartup(owner);
  expect(ensureStructuredResourceStartup(owner)).toBe(pending);
  await Promise.resolve(); expect(importStartup).toHaveBeenCalledOnce();
  first.resolve(deferred); await expect(pending).resolves.toBe(deferred);
  await Promise.resolve(); expect(importStartup).toHaveBeenCalledOnce();
  const retry = ensureStructuredResourceStartup(owner);
  expect(retry).not.toBe(pending); expect(ensureStructuredResourceStartup(owner)).toBe(retry);
  await Promise.resolve(); expect(importStartup).toHaveBeenCalledTimes(2);
  next.resolve(complete); await expect(retry).resolves.toBe(complete);
  expect(ensureStructuredResourceStartup(owner)).toBe(retry); expect(importStartup).toHaveBeenCalledTimes(2);
});

it.each([{}, {deferredPaperIds: []}, {deferredContextIds: ["writing-context"]}])("retains the original success cache when no paper is deferred: %j", async fields => {
  const result = {...complete, ...fields}, importStartup = vi.fn(async () => result), owner = runtime(importStartup);
  const pending = ensureStructuredResourceStartup(owner); await expect(pending).resolves.toBe(result);
  expect(ensureStructuredResourceStartup(owner)).toBe(pending); expect(importStartup).toHaveBeenCalledOnce();
});

it("keeps separate captured-runtime operations and never evicts another runtime's completed import", async () => {
  const delayed = pendingResult<StructuredStartupImportResult>(), firstImport = vi.fn(async () => delayed.promise);
  const secondImport = vi.fn(async () => complete), first = runtime(firstImport), second = runtime(secondImport);
  const before = ensureStructuredResourceStartup(first), after = ensureStructuredResourceStartup(second);
  await after; delayed.resolve(deferred); await before;
  expect(ensureStructuredResourceStartup(second)).toBe(after);
  firstImport.mockResolvedValueOnce(complete);
  const retry = ensureStructuredResourceStartup(first); expect(retry).not.toBe(before); await retry;
  expect(firstImport).toHaveBeenCalledTimes(2); expect(secondImport).toHaveBeenCalledOnce();
});

it("retains the original failed-attempt rejection and allows a new explicit retry", async () => {
  const failure = new Error("Original startup failure");
  const importStartup = vi.fn<StructuredResourceGateway["importStartup"]>().mockRejectedValueOnce(failure).mockResolvedValueOnce(complete);
  const owner = runtime(importStartup), rejected = ensureStructuredResourceStartup(owner);
  await expect(rejected).rejects.toBe(failure);
  const retry = ensureStructuredResourceStartup(owner); expect(retry).not.toBe(rejected);
  await expect(retry).resolves.toBe(complete); expect(ensureStructuredResourceStartup(owner)).toBe(retry);
  expect(importStartup).toHaveBeenCalledTimes(2);
});

it("keeps the production startup gate's readiness after a partial receipt without polling or another UI", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const first = pendingResult<StructuredStartupImportResult>(), importStartup = vi.fn(async () => first.promise), owner = runtime(importStartup);
  const mounted = vi.fn();
  function ExistingClient() {useEffect(mounted, []); return <div>original client</div>;}
  const host = document.createElement("div"), root = createRoot(host);
  try {
    await act(async () => root.render(<StructuredResourceStartupGate runtime={owner}><ExistingClient/></StructuredResourceStartupGate>));
    expect(mounted).not.toHaveBeenCalled(); expect(host.querySelector('[role="status"]')).not.toBeNull();
    await act(async () => first.resolve(deferred));
    expect(mounted).toHaveBeenCalledOnce(); expect(host.textContent).toBe("original client");
    expect(importStartup).toHaveBeenCalledOnce(); expect(host.querySelector("button")).toBeNull();
  } finally {await act(async () => root.unmount()); vi.unstubAllGlobals();}
});
