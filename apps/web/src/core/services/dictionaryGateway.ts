import {
  isDictionaryQuery, normalizeDictionaryQuery, stripDictionaryWord, dictionaryEditDistance,
  type DictionaryEntry, type DictionaryGateway, type DictionaryResult,
} from "@research-assistant/core/dictionary";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";

async function fetchDictionaryAsset(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  if (!isDesktopRuntime()) return globalThis.fetch(input, init);
  init?.signal?.throwIfAborted();
  const path = String(input);
  const prefix = "/dictionary/ecdict-v2/";
  if (!path.startsWith(prefix)) throw new Error("无效的词库资源路径。");
  const name = path.slice(prefix.length);
  let text: unknown;
  try { text = await invoke<unknown>("read_dictionary_asset", { name }); }
  catch (error) { throw new Error(typeof error === "string" ? error : "本地词库不可用，请重新准备词库。"); }
  init?.signal?.throwIfAborted();
  if (typeof text !== "string") throw new Error("本地词库响应格式无效。");
  return new Response(text);
}

interface DictionarySlot {
  readonly entries: readonly DictionaryEntry[];
  readonly lemmas: readonly string[];
  readonly variants: readonly string[];
}

interface Manifest {
  readonly format: number;
  readonly sharding: string;
  readonly files: Readonly<Record<string, { readonly bytes: number; readonly sha256: string }>>;
  readonly suggestions?: Readonly<Record<string, { readonly bytes: number; readonly sha256: string }>>;
}

function decodeFile(file: unknown): { readonly bytes: number; readonly sha256: string } {
  if (!isRecord(file) || !Number.isInteger(file.bytes) || typeof file.bytes !== "number" || file.bytes < 1 || file.bytes > 2_000_000
    || typeof file.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(file.sha256)) throw new Error("本地词库索引损坏。");
  return { bytes: file.bytes, sha256: file.sha256 };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function decodeManifest(value: unknown): Manifest {
  if (!isRecord(value) || value.format !== 1 || value.schema !== "entry-tuples-v1" || value.sharding !== "sha256-prefix-2" || !isRecord(value.files)) {
    throw new Error("本地词库版本不兼容，请重新准备词库。");
  }
  const files: Record<string, { bytes: number; sha256: string }> = {};
  for (const [key, file] of Object.entries(value.files)) {
    if (!/^[0-9a-f]{2}$/.test(key)) throw new Error("本地词库索引损坏。");
    files[key] = decodeFile(file);
  }
  let suggestions: Record<string, { bytes: number; sha256: string }> | undefined;
  if (value.suggestions !== undefined) {
    if (!isRecord(value.suggestions) || value.suggestions.schema !== "strip-and-spelling-v1" || !isRecord(value.suggestions.files)) throw new Error("本地模糊匹配索引损坏。");
    suggestions = {};
    for (const [name, file] of Object.entries(value.suggestions.files)) {
      if (!/^spelling-(0[3-9]|[12][0-9]|3[0-2])\.json$/.test(name)) throw new Error("本地模糊匹配索引名称无效。");
      suggestions[name] = decodeFile(file);
    }
  }
  return { format: 1, sharding: "sha256-prefix-2", files, ...(suggestions ? { suggestions } : {}) };
}

function decodeSlot(value: unknown): DictionarySlot {
  if (!isRecord(value) || !Array.isArray(value.e) || !Array.isArray(value.l)
    || value.l.length > 32 || !value.l.every(word => typeof word === "string" && word.length <= 200)) {
    throw new Error("本地词条数据损坏。");
  }
  if (value.s !== undefined && (!Array.isArray(value.s) || value.s.length > 32 || !value.s.every(word => typeof word === "string" && word.length <= 100))) throw new Error("本地形式匹配索引损坏。");
  const entries = value.e.map((entry: unknown): DictionaryEntry => {
    if (!Array.isArray(entry) || entry.length !== 5 || !entry.every(field => typeof field === "string")) throw new Error("本地词条数据损坏。");
    return { word: entry[0] as string, phonetic: entry[1] as string, translation: entry[2] as string,
      definition: entry[3] as string, exchange: entry[4] as string };
  });
  return { entries, lemmas: value.l as string[], variants: (value.s ?? []) as string[] };
}

async function digest(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, "0")).join("");
}

/** Bundled browser/native lexicon assets; at most four decoded shards per owner. */
export function createDictionaryGateway(
  fetchAsset: typeof fetch = fetchDictionaryAsset,
  baseUrl = "/dictionary/ecdict-v2/",
): DictionaryGateway<AbortSignal> {
  let manifest: Manifest | undefined;
  const cache = new Map<string, Record<string, unknown>>();
  const spellingCache = new Map<string, readonly (readonly [string, number])[]>();

  async function readAsset(path: string, signal?: AbortSignal): Promise<Uint8Array<ArrayBuffer>> {
    const response = await fetchAsset(`${baseUrl}${path}`, { ...(signal ? { signal } : {}) });
    if (!response.ok) throw new Error("本地词库不可用，请重新准备词库后重试。");
    return new Uint8Array(await response.arrayBuffer());
  }

  async function slot(word: string, signal?: AbortSignal): Promise<DictionarySlot | undefined> {
    signal?.throwIfAborted();
    if (!manifest) {
      const bytes = await readAsset("manifest.json", signal);
      if (bytes.length > 100_000) throw new Error("本地词库索引过大。");
      manifest = decodeManifest(JSON.parse(new TextDecoder().decode(bytes)) as unknown);
    }
    const key = (await digest(new TextEncoder().encode(word))).slice(0, 2);
    const file = manifest.files[key];
    if (!file) return undefined;
    let data = cache.get(key);
    if (!data) {
      const bytes = await readAsset(`${key}.json`, signal);
      if (bytes.length !== file.bytes || await digest(bytes) !== file.sha256) throw new Error("本地词库校验失败，请重新准备词库。");
      const decoded: unknown = JSON.parse(new TextDecoder().decode(bytes));
      if (!isRecord(decoded)) throw new Error("本地词库格式无效。");
      data = decoded;
    }
    signal?.throwIfAborted();
    cache.delete(key);
    cache.set(key, data);
    if (cache.size > 4) {
      const oldest = cache.keys().next().value;
      if (oldest !== undefined) cache.delete(oldest);
    }
    return Object.hasOwn(data, word) ? decodeSlot(data[word]) : undefined;
  }

  async function spellingRows(length: number, signal?: AbortSignal): Promise<readonly (readonly [string, number])[]> {
    const name = `spelling-${String(length).padStart(2, "0")}.json`, file = manifest?.suggestions?.[name];
    if (!file) return [];
    let rows = spellingCache.get(name);
    if (!rows) {
      const bytes = await readAsset(name, signal);
      if (bytes.length !== file.bytes || await digest(bytes) !== file.sha256) throw new Error("本地拼写候选索引校验失败，请重新准备词库。");
      const value: unknown = JSON.parse(new TextDecoder().decode(bytes));
      if (!Array.isArray(value) || value.length > 100_000 || !value.every(row => Array.isArray(row) && row.length === 2
        && typeof row[0] === "string" && /^[a-z]{3,32}$/.test(row[0]) && row[0].length === length && typeof row[1] === "number" && Number.isFinite(row[1]) && row[1] > 0)) throw new Error("本地拼写候选索引格式无效。");
      rows = value as [string, number][];
    }
    signal?.throwIfAborted(); spellingCache.delete(name); spellingCache.set(name, rows);
    if (spellingCache.size > 4) { const oldest = spellingCache.keys().next().value; if (oldest !== undefined) spellingCache.delete(oldest); }
    return rows;
  }

  async function suggest(query: string, match: DictionarySlot | undefined, entries: readonly DictionaryEntry[], signal?: AbortSignal): Promise<NonNullable<DictionaryResult["suggestions"]>> {
    const maximum = entries.length > 0 ? 3 : 8;
    const excluded = new Set([query, ...entries.map(entry => normalizeDictionaryQuery(entry.word))]);
    const stripped = stripDictionaryWord(query);
    const form = stripped === query ? match : await slot(stripped, signal);
    const forms = [...new Set([...(form?.entries.map(entry => normalizeDictionaryQuery(entry.word)) ?? []), ...(form?.variants ?? [])])]
      .filter(word => !excluded.has(word) && isDictionaryQuery(word) && stripDictionaryWord(word) === stripped).sort();
    const result: { word: string; kind: "form" | "spelling" }[] = forms.slice(0, maximum).map(word => ({ word, kind: "form" }));
    if (result.length >= maximum || !/^[a-z]{3,32}$/.test(query)) return result;
    const limit = query.length >= 7 ? 2 : 1;
    const candidates: { word: string; distance: number; rank: number }[] = [];
    let inspected = 0;
    for (let length = Math.max(3, query.length - limit); length <= Math.min(32, query.length + limit); length++) {
      for (const [word, rank] of await spellingRows(length, signal)) {
        const distance = dictionaryEditDistance(query, word, limit);
        if (distance <= limit && !excluded.has(word) && !forms.includes(word)) candidates.push({ word, distance, rank });
        // Cooperatively yield so selection replacement can cancel a large scan.
        if (++inspected % 2048 === 0) { await new Promise<void>(resolve => setTimeout(resolve, 0)); signal?.throwIfAborted(); }
      }
    }
    candidates.sort((a, b) => a.distance - b.distance || a.rank - b.rank || a.word.localeCompare(b.word, "en"));
    result.push(...candidates.slice(0, maximum - result.length).map(candidate => ({ word: candidate.word, kind: "spelling" as const })));
    signal?.throwIfAborted();
    return result;
  }

  return {
    async lookup(text, signal): Promise<DictionaryResult> {
      const query = normalizeDictionaryQuery(text);
      if (!isDictionaryQuery(query)) throw new Error("请选择一个英文单词或短语。");
      const match = await slot(query, signal);
      const entries = [...(match?.entries ?? [])];
      for (const lemma of match?.lemmas ?? []) {
        if (lemma === query) continue;
        const base = await slot(lemma, signal);
        for (const entry of base?.entries ?? []) {
          if (!entries.some(item => item.word === entry.word && item.translation === entry.translation && item.definition === entry.definition)) entries.push(entry);
        }
      }
      signal?.throwIfAborted();
      return { query, entries, suggestions: await suggest(query, match, entries, signal) };
    },
  };
}
