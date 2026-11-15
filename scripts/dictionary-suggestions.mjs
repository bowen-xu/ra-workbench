import { createHash } from "node:crypto";
import { writeFile } from "node:fs/promises";
import { resolve } from "node:path";

export const SUGGESTION_SCHEMA = "strip-and-spelling-v1";
export function stripWord(word) { return word.replace(/[^\p{L}\p{N}]/gu, "").toLowerCase(); }
export function spellingRank(row, word) {
  if (!/^[a-z]{3,32}$/.test(word)) return undefined;
  const frequency = Math.min(...[Number(row.frq), Number(row.bnc)].filter(value => value > 0));
  const stars = Number(row.collins);
  if (frequency > 50_000 && stars <= 0 && Number(row.oxford) !== 1) return undefined;
  return Number.isFinite(frequency) ? frequency : 50_001 + Math.max(0, 5 - stars);
}
export async function writeSpellingIndex(words, directory) {
  const groups = new Map();
  for (const [word, rank] of words) {
    const rows = groups.get(word.length) ?? []; rows.push([word, rank]); groups.set(word.length, rows);
  }
  const files = {};
  for (const [length, rows] of [...groups].sort(([a], [b]) => a - b)) {
    rows.sort((a, b) => a[1] - b[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
    const name = `spelling-${String(length).padStart(2, "0")}.json`, json = JSON.stringify(rows);
    files[name] = { bytes: Buffer.byteLength(json), sha256: createHash("sha256").update(json).digest("hex") };
    await writeFile(resolve(directory, name), json);
  }
  return { schema: SUGGESTION_SCHEMA, spellingWordCount: words.size, files };
}
