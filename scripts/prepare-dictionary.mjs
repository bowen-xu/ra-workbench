import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parse } from "csv-parse";
import { extractDictionaryArchive } from "./extract-dictionary-archive.mjs";
import { SUGGESTION_SCHEMA, spellingRank, stripWord, writeSpellingIndex } from "./dictionary-suggestions.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const ECDICT_REF = "bc015ed2e24a7abef49fc6dbbb7fe32c1dadaf8b";
const CSV_SHA256 = "1a6947e04785db63613a92e14903cdae7954f7e84860b10e68e5c7cbb3f9c3cf";
const ARCHIVE_SHA256 = "f370a0ecb58ada758d9dfe739db1667fd4ed87ed3055a4a7cb6c7054ecdf83d6";
const EXPANDED_CSV_SHA256 = "88fce01e0a30524192a62e363d47eeb036fa17820d5826121b3b419fd67a3996";
const CORPUS = "stardict-with-base-fallback-v1";
const LICENSE_SHA256 = "f8552dd246f61a4e064569eae6194a01c6b3d63b03bf27c6ca863593c549ed0f";
const FORMAT = 1;

function normalize(text) {
  return text.normalize("NFKC").replace(/[‘’]/g, "'").replace(/[‐‑–]/g, "-")
    .trim().replace(/\s+/g, " ").toLowerCase();
}

function shardKey(word) {
  return createHash("sha256").update(word, "utf8").digest("hex").slice(0, 2);
}

/** CSV is a build input; no Python, CSV parser or upstream network runs in the app. */
export async function buildDictionary(csvPath, outputDirectory, licenseText, { fallbackCsvPath } = {}) {
  const shards = new Map();
  const spellingWords = new Map();
  function slot(word) {
    const key = shardKey(word);
    if (!shards.has(key)) shards.set(key, Object.create(null));
    const shard = shards.get(key);
    if (!Object.hasOwn(shard, word)) shard[word] = { e: [], l: [] };
    return shard[word];
  }
  let entryCount = 0;
  let primaryEntryCount = 0;
  let fallbackEntryCount = 0;
  for (const [index, path] of [csvPath, ...(fallbackCsvPath ? [fallbackCsvPath] : [])].entries()) {
    const parser = createReadStream(path).pipe(parse({ columns: true, bom: true, skip_empty_lines: true }));
    for await (const row of parser) {
      const word = normalize(row.word);
      if (!word) throw new Error("ECDICT contains an empty word");
      if (index > 0 && shards.get(shardKey(word))?.[word]?.e.length > 0) continue;
      const entry = Object.fromEntries(["word", "phonetic", "translation", "definition", "exchange"]
        .map(field => [field, row[field] ?? ""]));
      slot(word).e.push([entry.word, entry.phonetic, entry.translation, entry.definition, entry.exchange]);
      const stripped = stripWord(word);
      if (stripped && stripped !== word && /^[a-z0-9][a-z0-9 '\-./()]*$/i.test(word) && word.length <= 100) {
        const variants = slot(stripped).s ??= [];
        if (variants.length < 32 && !variants.includes(word)) variants.push(word);
      }
      const rank = spellingRank(row, word);
      if (rank !== undefined) spellingWords.set(word, Math.min(rank, spellingWords.get(word) ?? Infinity));
      for (const part of entry.exchange.split("/")) {
        const colon = part.indexOf(":");
        const type = part.slice(0, colon);
        if (!["p", "d", "i", "3", "r", "t", "s", "0"].includes(type) || colon < 1) continue;
        for (const form of part.slice(colon + 1).split(",")) {
          const target = normalize(form);
          if (!target || target === word) continue;
          const from = type === "0" ? word : target;
          const lemma = type === "0" ? target : word;
          const links = slot(from).l;
          if (!links.includes(lemma)) links.push(lemma);
        }
      }
      entryCount++;
      if (index === 0) primaryEntryCount++;
      else fallbackEntryCount++;
    }
  }
  await mkdir(outputDirectory, { recursive: true });
  const files = {};
  for (const [key, data] of shards) {
    const json = JSON.stringify(data);
    files[key] = { bytes: Buffer.byteLength(json), sha256: createHash("sha256").update(json).digest("hex") };
    await writeFile(resolve(outputDirectory, `${key}.json`), json);
  }
  await writeFile(resolve(outputDirectory, "LICENSE.txt"), licenseText);
  const suggestions = await writeSpellingIndex(spellingWords, outputDirectory);
  const manifest = { format: FORMAT, source: "ECDICT", ref: ECDICT_REF, corpus: CORPUS,
    sourceSha256: EXPANDED_CSV_SHA256, sourceArchiveSha256: ARCHIVE_SHA256, fallbackSha256: CSV_SHA256,
    sharding: "sha256-prefix-2", schema: "entry-tuples-v1", entryCount, primaryEntryCount, fallbackEntryCount, files, suggestions };
  await writeFile(resolve(outputDirectory, "manifest.json"), JSON.stringify(manifest));
  return manifest;
}

async function verifiedInput(name, sha256) {
  const cache = resolve(ROOT, ".cache/dictionary", ECDICT_REF, name);
  const sourceCache = resolve(ROOT, "docs/references/projects/skywind3000-ecdict", name);
  let bytes;
  try { bytes = await readFile(cache); } catch (error) {
    if (error.code !== "ENOENT") throw error;
    try { bytes = await readFile(sourceCache); } catch (sourceError) {
      if (sourceError.code !== "ENOENT") throw sourceError;
      console.log(`Downloading pinned ECDICT ${name}`);
      const response = await fetch(`https://raw.githubusercontent.com/skywind3000/ECDICT/${ECDICT_REF}/${name}`, {
        signal: AbortSignal.timeout(120_000),
      });
      if (!response.ok) throw new Error(`ECDICT download failed (${response.status})`);
      bytes = Buffer.from(await response.arrayBuffer());
    }
  }
  if (createHash("sha256").update(bytes).digest("hex") !== sha256) throw new Error(`ECDICT ${name} checksum mismatch`);
  await mkdir(dirname(cache), { recursive: true });
  await writeFile(cache, bytes);
  return cache;
}

export async function prepareDictionary() {
  const destination = resolve(ROOT, "apps/web/public/dictionary/ecdict-v2");
  try {
    const manifest = JSON.parse(await readFile(resolve(destination, "manifest.json"), "utf8"));
    if (manifest && typeof manifest === "object" && manifest.files && typeof manifest.files === "object"
      && manifest.format === FORMAT && manifest.corpus === CORPUS && manifest.schema === "entry-tuples-v1" && manifest.ref === ECDICT_REF
      && manifest.sourceSha256 === EXPANDED_CSV_SHA256 && manifest.sourceArchiveSha256 === ARCHIVE_SHA256 && manifest.fallbackSha256 === CSV_SHA256
      && manifest.entryCount === 3403176 && manifest.primaryEntryCount === 3402564 && manifest.fallbackEntryCount === 612
      && manifest.suggestions?.schema === SUGGESTION_SCHEMA && typeof manifest.suggestions.files === "object"
      && Object.keys(manifest.files).length === 256) {
      let valid = true;
      for (const [name, file] of [...Object.entries(manifest.files).map(([key, file]) => [`${key}.json`, file]), ...Object.entries(manifest.suggestions.files)]) {
        if (!/^([0-9a-f]{2}|spelling-(0[3-9]|[12][0-9]|3[0-2]))\.json$/.test(name) || !file || typeof file.bytes !== "number" || typeof file.sha256 !== "string") { valid = false; break; }
        const bytes = await readFile(resolve(destination, name));
        if (bytes.length !== file.bytes || createHash("sha256").update(bytes).digest("hex") !== file.sha256) { valid = false; break; }
      }
      const licenseBytes = await readFile(resolve(destination, "LICENSE.txt"));
      if (valid && createHash("sha256").update(licenseBytes).digest("hex") === LICENSE_SHA256) return;
    }
  } catch (error) {
    if (error.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error;
  }
  const archive = await verifiedInput("stardict.7z", ARCHIVE_SHA256);
  const csv = await extractDictionaryArchive(archive, resolve(dirname(archive), "stardict.csv"), {
    archiveSha256: ARCHIVE_SHA256, csvSha256: EXPANDED_CSV_SHA256,
  });
  const fallbackCsvPath = await verifiedInput("ecdict.csv", CSV_SHA256);
  const license = await verifiedInput("LICENSE", LICENSE_SHA256);
  const temporary = `${destination}.preparing-${process.pid}`;
  try {
    const manifest = await buildDictionary(csv, temporary, await readFile(license, "utf8"), { fallbackCsvPath });
    if (manifest.primaryEntryCount !== 3402564 || manifest.fallbackEntryCount !== 612 || manifest.entryCount !== 3403176) throw new Error("ECDICT entry count mismatch");
    await rm(destination, { recursive: true, force: true });
    await rename(temporary, destination);
    await rm(resolve(ROOT, "apps/web/public/dictionary/ecdict-v1"), { recursive: true, force: true });
    console.log(`Prepared ECDICT: ${manifest.entryCount} entries, ${Object.keys(manifest.files).length} local shards`);
  } finally { await rm(temporary, { recursive: true, force: true }); }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { await prepareDictionary(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
