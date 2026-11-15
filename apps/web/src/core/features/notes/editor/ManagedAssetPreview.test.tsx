// @vitest-environment happy-dom
import {act, useLayoutEffect} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import type {NotesGateway} from "@research-assistant/data/notes";
import {NotesProvider} from "../NotesClient";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import {ManagedAssetPreview} from "./noteEditorSchema";

const bytes = Uint8Array.from([137,80,78,71]);
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const stop of cleanups.splice(0)) await stop(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
function pending<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => {resolve = done; reject = fail;});
  return {promise, resolve, reject};
}
function gateway(read: NotesGateway["readNoteAsset"]): NotesGateway {
  return new Proxy(noteRendererGateway, {get(target, key) {return key === "readNoteAsset" ? read : Reflect.get(target, key);}});
}
async function setup(read: NotesGateway["readNoteAsset"], image = true) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  let nextUrl = 0;
  const create = vi.spyOn(URL, "createObjectURL").mockImplementation(() => `blob:asset-${++nextUrl}`);
  const revoke = vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const owner = gateway(read), committed: {url: string | null; error: string | null}[] = [];
  let mounted = true;
  function Probe({id, mime, source}: {id: string; mime: string; source: NotesGateway}) {
    useLayoutEffect(() => {committed.push({url: host.querySelector("img")?.getAttribute("src") ?? null, error: host.querySelector("small")?.textContent ?? null});}, [id, mime, source]);
    return <NotesProvider gateway={source}><ManagedAssetPreview assetId={id} displayName="figure.png" mimeType={mime} caption="说明" isImage={image}/></NotesProvider>;
  }
  const render = async (id = "a", mime = "image/png", source = owner) => {
    await act(async () => root.render(<Probe id={id} mime={mime} source={source}/>));
  };
  const stop = async () => {if (mounted) {mounted = false; await act(async () => root.unmount()); host.remove();}};
  cleanups.push(stop);
  await render();
  return {host, render, stop, create, revoke, committed};
}
it("releases replaced URLs and hides the previous image in the same commit", async () => {
  const second = pending<Uint8Array>();
  const read = vi.fn((id: string) => id === "a" ? Promise.resolve(bytes) : second.promise);
  const f = await setup(read);
  expect(f.host.querySelector("img")?.getAttribute("src")).toBe("blob:asset-1");
  await f.render("b");
  expect(f.committed.at(-1)?.url).toBeNull();
  expect(f.host.querySelector("img")).toBeNull();
  expect(f.revoke).toHaveBeenCalledExactlyOnceWith("blob:asset-1");
  await act(async () => second.resolve(bytes));
  expect(f.host.querySelector("img")?.getAttribute("src")).toBe("blob:asset-2");
  await f.stop(); expect(f.revoke.mock.calls).toEqual([["blob:asset-1"],["blob:asset-2"]]);
});
it("keeps safe read diagnostics and clears an old failure as soon as the resource changes", async () => {
  const second = pending<Uint8Array>();
  const error = Object.assign(new Error("磁盘读取失败（EIO）"), {code: "storage_error"});
  const f = await setup(id => id === "a" ? Promise.reject(error) : second.promise);
  expect(f.host.textContent).toContain("storage_error"); expect(f.host.textContent).toContain("EIO");
  expect(f.host.textContent).not.toContain("文件缺失");
  await f.render("b");
  expect(f.committed.at(-1)?.error).toBeNull();
  expect(f.host.textContent).not.toContain("storage_error");
  await act(async () => second.resolve(bytes));
  expect(f.host.querySelector("img")).not.toBeNull(); expect(f.host.textContent).not.toContain("失败");
});
it("ignores an old successful request that finishes after a new resource", async () => {
  const first = pending<Uint8Array>();
  const f = await setup(id => id === "a" ? first.promise : Promise.resolve(bytes));
  await f.render("b"); expect(f.host.querySelector("img")?.getAttribute("src")).toBe("blob:asset-1");
  await act(async () => first.resolve(bytes));
  expect(f.create).toHaveBeenCalledOnce();
  expect(f.host.querySelector("img")?.getAttribute("src")).toBe("blob:asset-1");
});
it.each(["mime", "client"] as const)("does not show bytes from a previous %s owner while reloading", async change => {
  const loading = pending<Uint8Array>();
  const read = vi.fn().mockResolvedValueOnce(bytes).mockImplementation(() => loading.promise);
  const f = await setup(read);
  await f.render("a", change === "mime" ? "image/jpeg" : "image/png", change === "client" ? gateway(() => loading.promise) : undefined);
  expect(f.committed.at(-1)?.url).toBeNull(); expect(f.host.querySelector("img")).toBeNull();
  expect(f.revoke).toHaveBeenCalledExactlyOnceWith("blob:asset-1");
  await act(async () => loading.resolve(bytes));
  expect(f.host.querySelector("img")?.getAttribute("src")).toBe("blob:asset-2");
  expect((f.create.mock.calls.at(-1)![0] as Blob).type).toBe(change === "mime" ? "image/jpeg" : "image/png");
});
it("reports a decode error and releases the failed image URL exactly once", async () => {
  const f = await setup(async () => bytes);
  await act(async () => f.host.querySelector("img")!.dispatchEvent(new Event("error")));
  expect(f.host.querySelector("img")).toBeNull(); expect(f.host.textContent).toContain("note_asset_decode_failed");
  expect(f.host.textContent).not.toContain("文件缺失"); expect(f.revoke).toHaveBeenCalledExactlyOnceWith("blob:asset-1");
  await f.stop(); expect(f.revoke).toHaveBeenCalledOnce();
});
it("does not allocate a URL for a request that resolves after unmount", async () => {
  const loading = pending<Uint8Array>(), f = await setup(() => loading.promise);
  await f.stop(); await act(async () => loading.resolve(bytes));
  expect(f.create).not.toHaveBeenCalled(); expect(f.revoke).not.toHaveBeenCalled();
});
it("retains file download behavior without mounting an image decoder", async () => {
  const f = await setup(async () => bytes, false);
  expect(f.host.querySelector("img")).toBeNull();
  expect(f.host.querySelector("a")?.getAttribute("href")).toBe("blob:asset-1");
  expect(f.host.querySelector("a")?.getAttribute("download")).toBe("figure.png");
});
