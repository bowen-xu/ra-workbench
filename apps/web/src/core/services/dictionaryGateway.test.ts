import { createHash } from "node:crypto";
import { afterEach, describe, expect, it, vi } from "vitest";
vi.mock("../runtime/platformCore", () => ({ invoke: vi.fn(), isDesktopRuntime: vi.fn(() => false) }));
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { createDictionaryGateway } from "./dictionaryGateway";

function lexicon(words: Record<string, unknown>, spelling: readonly (readonly [string,number])[] = []) {
  const assets = new Map<string, string>();
  const shards = new Map<string, Record<string, unknown>>();
  for (const [word, entry] of Object.entries(words)) {
    const key = createHash("sha256").update(word).digest("hex").slice(0, 2);
    const shard = shards.get(key) ?? {};
    shard[word] = entry; shards.set(key, shard);
  }
  const files = Object.fromEntries([...shards].map(([key, shard]) => {
    const json = JSON.stringify(shard); assets.set(`/dictionary/${key}.json`, json);
    return [key, { bytes: Buffer.byteLength(json), sha256: createHash("sha256").update(json).digest("hex") }];
  }));
  const spellingFiles: Record<string,{bytes:number;sha256:string}> = {};
  for (const length of new Set(spelling.map(([word])=>word.length))) {
    const name=`spelling-${String(length).padStart(2,"0")}.json`,json=JSON.stringify(spelling.filter(([word])=>word.length===length));
    assets.set(`/dictionary/${name}`,json);spellingFiles[name]={bytes:Buffer.byteLength(json),sha256:createHash("sha256").update(json).digest("hex")};
  }
  assets.set("/dictionary/manifest.json", JSON.stringify({ format: 1, sharding: "sha256-prefix-2", schema: "entry-tuples-v1", files,
    suggestions:{schema:"strip-and-spelling-v1",files:spellingFiles} }));
  const fetchAsset = vi.fn(async (url: RequestInfo | URL) => {
    const text = assets.get(String(url));
    return new Response(text ?? "not found", { status: text ? 200 : 404 });
  });
  return { assets, fetchAsset, gateway: createDictionaryGateway(fetchAsset, "/dictionary/") };
}

describe("local ECDICT gateway", () => {
  afterEach(() => { vi.mocked(isDesktopRuntime).mockReturnValue(false); vi.mocked(invoke).mockReset(); });
  it("uses restricted native resource IPC on desktop without sending the selected text", async () => {
    const { assets } = lexicon({ cat: { e: [["cat", "", "猫", "", ""]], l: [] } });
    vi.mocked(isDesktopRuntime).mockReturnValue(true);
    vi.mocked(invoke).mockImplementation(async (_command, args) => assets.get(`/dictionary/${String(args?.name)}`));
    expect((await createDictionaryGateway().lookup("cat")).entries[0]?.translation).toBe("猫");
    expect(invoke).toHaveBeenCalledTimes(2);
    for (const [command, args] of vi.mocked(invoke).mock.calls) {
      expect(command).toBe("read_dictionary_asset");
      expect(Object.keys(args ?? {})).toEqual(["name"]);
      expect(args?.name).toMatch(/^(manifest|[0-9a-f]{2})\.json$/);
    }
  });
  it("looks up irregular and ambiguous forms with exact entries first and no recursive alias cycle", async () => {
    const { gateway, fetchAsset } = lexicon({
      went: { e: [["went", "went", "go 的过去式", "", "0:go"]], l: ["go"] },
      go: { e: [["go", "ɡəʊ", "v. 去", "move", "p:went"]], l: ["went"] },
      axes: { e: [], l: ["ax", "axis"] },
      ax: { e: [["ax", "", "斧头", "", "s:axes"]], l: [] },
      axis: { e: [["axis", "", "轴", "", "s:axes"]], l: [] },
    });
    expect((await gateway.lookup("“Went,”")).entries.map(entry => entry.word)).toEqual(["went", "go"]);
    expect((await gateway.lookup("axes")).entries.map(entry => entry.word)).toEqual(["ax", "axis"]);
    expect(fetchAsset.mock.calls.every(([url]) => String(url).startsWith("/dictionary/"))).toBe(true);
  });
  it("caches at most four shards, reuses the manifest and avoids requests for invalid queries", async () => {
    const words = ["cat", "dog", "horse", "bird", "fish", "mouse"];
    const { gateway, fetchAsset } = lexicon(Object.fromEntries(words.map(word => [word, { e: [[word, "", "意思", "", ""]], l: [] }])));
    await expect(gateway.lookup("a b c d e f g")).rejects.toThrow("请选择");
    expect(fetchAsset).not.toHaveBeenCalled();
    await gateway.lookup("cat"); await gateway.lookup("cat");
    expect(fetchAsset).toHaveBeenCalledTimes(2);
    for (const word of words.slice(1)) await gateway.lookup(word);
    const before = fetchAsset.mock.calls.length;
    await gateway.lookup("cat"); expect(fetchAsset).toHaveBeenCalledTimes(before + 1);
    expect(fetchAsset.mock.calls.filter(([url]) => String(url).endsWith("manifest.json"))).toHaveLength(1);
  });
  it("distinguishes unknown words, missing assets and corrupt data; permits retry", async () => {
    const { gateway, assets } = lexicon({ cat: { e: [["cat", "", "猫", "", ""]], l: [] } });
    expect((await gateway.lookup("nonexistentword")).entries).toEqual([]);
    const key = createHash("sha256").update("cat").digest("hex").slice(0, 2);
    const good = assets.get(`/dictionary/${key}.json`);
    assets.delete(`/dictionary/${key}.json`);
    await expect(gateway.lookup("cat")).rejects.toThrow("不可用");
    assets.set(`/dictionary/${key}.json`, "corrupt");
    await expect(gateway.lookup("cat")).rejects.toThrow("校验失败");
    assets.set(`/dictionary/${key}.json`, good ?? "");
    expect((await gateway.lookup("cat")).entries[0]?.translation).toBe("猫");
  });
  it("never fetches or returns a result after cancellation", async () => {
    const { gateway, fetchAsset } = lexicon({});
    const controller = new AbortController(); controller.abort();
    await expect(gateway.lookup("cat", controller.signal)).rejects.toThrow();
    expect(fetchAsset).not.toHaveBeenCalled();
  });
  it("offers strip variants and ranks misspellings without adopting their definitions", async () => {
    const {gateway}=lexicon({
      longtime:{e:[],l:[],s:["long-time","long time"]},
      "long-time":{e:[["long-time","","长期","",""]],l:[]},
      work:{e:[["work","","工作","",""]],l:[]},
    },[["work",10],["wore",20],["word",30],["recognition",100],["recondition",200]]);
    expect((await gateway.lookup("long--time"))).toMatchObject({entries:[],suggestions:[{word:"long time",kind:"form"},{word:"long-time",kind:"form"}]});
    expect((await gateway.lookup("wrok"))).toMatchObject({entries:[],suggestions:[{word:"work",kind:"spelling"}]});
    expect((await gateway.lookup("workk")).suggestions?.[0]?.word).toBe("work");
    expect((await gateway.lookup("recongition")).suggestions?.[0]?.word).toBe("recognition");
    expect(await gateway.lookup("work")).toMatchObject({entries:[{word:"work",translation:"工作"}],suggestions:[{word:"wore",kind:"spelling"},{word:"word",kind:"spelling"}]});
  });
  it("limits successful lookups to three suggestions and misses to eight, excluding displayed headwords", async () => {
    const words = ["word", "wore", "worn", "worm", "worb", "worx", "worq", "worj", "worh"];
    const { gateway } = lexicon({ work: { e: [["work", "", "工作", "", ""]], l: [] } },
      [["work", 1], ...words.map((word, index): readonly [string, number] => [word, index + 2])]);
    const hit = await gateway.lookup("work");
    expect(hit.entries[0]?.translation).toBe("工作");
    expect(hit.suggestions?.map(candidate => candidate.word)).toEqual(words.slice(0, 3));
    expect((await gateway.lookup("worz")).suggestions).toHaveLength(8);
    const forms = lexicon({
      "long-time": { e: [["long-time", "", "长期", "", ""]], l: [] },
      longtime: { e: [], l: [], s: ["long-time", "long time"] },
      works: { e: [], l: ["work"] },
      work: { e: [["work", "", "工作", "", ""]], l: [] },
    }, [["work", 1], ["words", 2]]);
    expect((await forms.gateway.lookup("long-time")).suggestions).toEqual([{word:"long time",kind:"form"}]);
    expect((await forms.gateway.lookup("works")).suggestions).toEqual([{word:"words",kind:"spelling"}]);
    expect((await forms.gateway.lookup("unrelated")).suggestions).toEqual([]);
  });
  it("rejects corrupted candidate assets and stops a scan when its owner cancels", async () => {
    const {gateway,assets}=lexicon({},[["work",10]]);assets.set("/dictionary/spelling-04.json","[]");
    await expect(gateway.lookup("wrok")).rejects.toThrow("校验失败");
    const controller=new AbortController();const lex=lexicon({},[["work",10]]);
    const fetchAsset=vi.fn(async(url:RequestInfo|URL)=>{if(String(url).includes("spelling-"))controller.abort();return lex.fetchAsset(url);});
    await expect(createDictionaryGateway(fetchAsset,"/dictionary/").lookup("wrok",controller.signal)).rejects.toThrow();
  });
});
