import type { PdfCharacterSelection, PdfPageCharacterModel } from "./pdfPageCharacterModel";
import { beforePdfCharacter, createPdfCharacterSelection } from "./pdfPageCharacterSelection";

// PDF glyphs can encode Latin presentation ligatures while extraction spells out their letters.
// Keep scientific symbols, case and punctuation intact rather than applying general compatibility folding.
const latinLigatures: Readonly<Record<string, string>> = { "ﬀ": "ff", "ﬁ": "fi", "ﬂ": "fl", "ﬃ": "ffi", "ﬄ": "ffl", "ﬅ": "st", "ﬆ": "st" };
function normalizeTypography(value: string): string {
  return value.normalize("NFC").replace(/[\uFB00-\uFB06]/gu, glyph => latinLigatures[glyph] ?? glyph);
}

/** Matches whitespace and Latin ligatures. Ambiguous quotes require exact surrounding context. */
export function selectPdfQuote(model: PdfPageCharacterModel, quote: string, prefix = "", suffix = ""): PdfCharacterSelection | undefined {
  const characters: number[] = [];
  let text = "";
  model.chars.forEach((character, index) => {
    for (const value of normalizeTypography(character.value)) {
      if (/\s/u.test(value)) continue;
      text += value;
      for (let unit = 0; unit < value.length; unit += 1) characters.push(index);
    }
  });
  const normalize = (value: string): string => normalizeTypography(value).replace(/\s/gu, "");
  const target = normalize(quote);
  if (!target) return undefined;
  const matches: number[] = [];
  let offset = text.indexOf(target);
  while (offset >= 0) { matches.push(offset); offset = text.indexOf(target, offset + 1); }
  const candidates = matches.length <= 1 ? matches : matches.filter(index =>
    (prefix === "" || text.slice(0, index).endsWith(normalize(prefix)))
    && (suffix === "" || text.slice(index + target.length).startsWith(normalize(suffix))));
  const start = candidates[0];
  if (candidates.length !== 1 || start === undefined) return undefined;
  const first = characters[start]; const last = characters[start + target.length - 1];
  if (first === undefined || last === undefined) return undefined;
  return createPdfCharacterSelection(model, beforePdfCharacter(first), { index: last, offset: 1 });
}
