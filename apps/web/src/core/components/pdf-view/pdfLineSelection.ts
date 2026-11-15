import type { SourceLineRange } from "@research-assistant/core/source";
import type { PdfPageCharacterModel } from "./pdfPageCharacterModel";
import { beforePdfCharacter, createPdfCharacterSelection } from "./pdfPageCharacterSelection";

/** PDF columns count the same page characters as selection source locations. */
export function selectPdfLines(model: PdfPageCharacterModel, range: SourceLineRange) {
  const start = model.lines[range.startLine - 1]?.characterIndexes[range.startColumn - 1];
  const end = model.lines[range.endLine - 1]?.characterIndexes[range.endColumn - 1];
  if (start === undefined || end === undefined || start > end) return undefined;
  return createPdfCharacterSelection(model, beforePdfCharacter(start), { index: end, offset: 1 });
}
