import type {DOMOutputSpec, NodeSpec} from "@tiptap/pm/model";

export interface RestrictedTextConstraint {
  readonly maxLength?: number;
  readonly singleLine?: boolean;
}

/** Reject invalid edits; callers must not silently trim an existing value. */
export function legalRestrictedText(text: string, constraint: RestrictedTextConstraint = {}): boolean {
  return (constraint.maxLength === undefined || text.length <= constraint.maxLength)
    && (!constraint.singleLine || !/[\r\n]/u.test(text));
}

/** Normalize initial/plain clipboard text, never a replacement for edit validation.
 * Length follows native UTF-16 offsets without splitting a surrogate pair. */
export function normalizeRestrictedText(text: string, constraint: RestrictedTextConstraint = {}): string {
  const line = constraint.singleLine ? text.replace(/[\r\n]+/gu, " ") : text;
  if (constraint.maxLength === undefined || line.length <= constraint.maxLength) return line;
  const value = line.slice(0, constraint.maxLength);
  return /[\uD800-\uDBFF]$/u.test(value) && /^[\uDC00-\uDFFF]/u.test(line.slice(constraint.maxLength))
    ? value.slice(0, -1) : value;
}

/** A native plain-text projection. No formatting marks or secondary history. */
export function restrictedTextNodeSpec({tag, className, label, multiline = false}: {
  readonly tag: string;
  readonly className?: string;
  readonly label?: string;
  readonly multiline?: boolean;
}) {
  const attributes = {"data-restricted-block-text": "true", ...(className ? {class: className} : {}), ...(label ? {"aria-label": label} : {})};
  return {
    content: "text*", marks: "", code: multiline, whitespace: "pre",
    parseDOM: [{tag, preserveWhitespace: "full"}],
    toDOM: (): DOMOutputSpec => [tag, attributes, 0],
  } satisfies NodeSpec;
}
