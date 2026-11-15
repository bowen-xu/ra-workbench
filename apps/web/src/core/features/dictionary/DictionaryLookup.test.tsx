/** @vitest-environment happy-dom */
import { StrictMode, act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DictionaryGateway, DictionaryResult } from "@research-assistant/core/dictionary";
import { DictionaryLookup } from "./DictionaryLookup";
import { notify } from "../../services/notifications";

describe("dictionary selection panel", () => {
  let host: HTMLDivElement, root: Root;
  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });
  async function render(text: string, gateway: DictionaryGateway<AbortSignal>) {
    await act(async () => root.render(<StrictMode><DictionaryLookup text={text} gateway={gateway} context={{word:text,before:"",after:""}} /></StrictMode>));
  }
  it("queries locally, presents definitions as text and allows a linked word form lookup", async () => {
    const lookup = vi.fn(async (query: string): Promise<DictionaryResult> => ({ query, entries: [{ word: "go", phonetic: "ɡəʊ",
      translation: "v. 去\\n<em>走</em>", definition: "move", exchange: "p:went" }] }));
    await render("go", { lookup });
    expect(host.textContent).toContain("v. 去"); expect(host.textContent).toContain("<em>走</em>");
    expect(host.querySelector("em")).toBeNull();
    const form = [...host.querySelectorAll("button")].find(button => button.textContent === "过去式 · went");
    await act(async () => form?.click());
    expect(lookup).toHaveBeenLastCalledWith("went", expect.any(AbortSignal));
    expect(host.querySelector("input")).toBeNull(); expect(host.querySelector("form")).toBeNull();
  });
  it("does not load a sentence or fall back to AI", async () => {
    const lookup = vi.fn(async (query: string) => ({ query, entries: [] }));
    await render("This is a long sentence which should not be sent anywhere.", { lookup });
    expect(lookup).not.toHaveBeenCalled(); expect(host.textContent).toContain("英文单词或短语");
  });
  it("ignores a stale result when selection changes, even if the adapter ignores cancellation", async () => {
    const pending: { query: string; signal?: AbortSignal; resolve: (result: DictionaryResult) => void }[] = [];
    const gateway: DictionaryGateway<AbortSignal> = { lookup: (query, signal) => new Promise(resolve => pending.push({ query, ...(signal ? { signal } : {}), resolve })) };
    await render("old", gateway); await render("new", gateway);
    expect(pending.filter(item => item.query === "old").every(item => item.signal?.aborted)).toBe(true);
    await act(async () => {
      pending.filter(item => item.query === "new").forEach(item => item.resolve({ query: "new", entries: [] }));
      pending.filter(item => item.query === "old").forEach(item => item.resolve({ query: "old", entries: [] }));
    });
    expect(host.textContent).toContain("未找到“new”"); expect(host.textContent).not.toContain("未找到“old”");
  });
  it("retains the query after failure and retries only on a user action", async () => {
    const error = vi.spyOn(notify, "error").mockImplementation(() => "toast");
    const gateway = { lookup: vi.fn().mockRejectedValue(new Error("词库不可用")) };
    await render("cat", gateway);
    expect(error).toHaveBeenCalledWith("词库不可用"); expect(host.querySelector("input")).toBeNull();
    const calls = gateway.lookup.mock.calls.length;
    gateway.lookup.mockResolvedValue({ query: "cat", entries: [] });
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "重试查词")?.click());
    expect(gateway.lookup).toHaveBeenCalledTimes(calls + 1); expect(host.textContent).toContain("未找到“cat”");
  });
  it("shows optional candidates after failure and queries them only after a click", async () => {
    const lookup = vi.fn(async (query:string):Promise<DictionaryResult> => query === "wrok"
      ? {query,entries:[],suggestions:[{word:"work",kind:"spelling"}]}
      : {query,entries:[{word:"work",phonetic:"",translation:"工作",definition:"",exchange:""}]});
    await render("wrok",{lookup});
    expect(host.textContent).toContain("未找到“wrok”"); expect(host.textContent).toContain("你可能想查");
    expect(host.querySelector("article")).toBeNull(); expect(lookup.mock.calls.every(([query])=>query==="wrok")).toBe(true);
    await act(async()=>host.querySelector<HTMLButtonElement>(".dictionary-suggestions button")?.click());
    expect(lookup).toHaveBeenLastCalledWith("work",expect.any(AbortSignal)); expect(host.textContent).toContain("工作");
    expect(host.querySelector(".dictionary-suggestions")).toBeNull();
  });
  it("shows candidates alongside an exact definition without automatically changing the result", async () => {
    const lookup = vi.fn(async (query: string): Promise<DictionaryResult> => ({ query,
      entries: [{word:query,phonetic:"",translation:query === "work" ? "工作" : "单词",definition:"",exchange:""}],
      suggestions: query === "work" ? [{word:"word",kind:"spelling"},{word:"wore",kind:"spelling"},{word:"worm",kind:"spelling"}] : [] }));
    await render("work", {lookup});
    expect(host.querySelector("article strong")?.textContent).toBe("work");
    expect(host.textContent).toContain("工作");
    expect(host.querySelectorAll(".dictionary-suggestions button")).toHaveLength(3);
    expect(lookup.mock.calls.every(([query]) => query === "work")).toBe(true);
    await act(async () => host.querySelector<HTMLButtonElement>(".dictionary-suggestions button")?.click());
    expect(lookup).toHaveBeenLastCalledWith("word", expect.any(AbortSignal));
    expect(host.querySelector("article strong")?.textContent).toBe("word");
    expect(host.querySelector(".dictionary-suggestions")).toBeNull();
  });
});
