import { StreamLanguage } from "@codemirror/language";
import { stex } from "@codemirror/legacy-modes/mode/stex";
import type { Extension } from "@codemirror/state";

export function latexLanguage(): Extension {
  return StreamLanguage.define(stex);
}
