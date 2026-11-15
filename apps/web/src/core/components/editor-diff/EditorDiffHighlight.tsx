import type { CSSProperties, ReactNode } from "react";
import "./EditorDiffHighlight.css";

/** Instance-local presentation only. Matching, editor nodes and history remain
 * owned by the renderer. Omitted values inherit the enclosing appearance. */
export interface EditorDiffHighlightAppearance {
  readonly addedLine?: string;
  readonly removedLine?: string;
  readonly incomingLine?: string;
  readonly addedText?: string;
  readonly removedText?: string;
  readonly incomingText?: string;
  readonly incomingForeground?: string;
  readonly addedUnderline?: string;
  readonly addedMarker?: string;
  readonly removedMarker?: string;
  readonly incomingMarker?: string;
  readonly underlineThickness?: string;
}

function appearanceStyle(appearance: EditorDiffHighlightAppearance | undefined): CSSProperties {
  const style: Record<string, string> = {};
  for (const [key, value] of Object.entries(appearance ?? {})) {
    if (value !== undefined) style[`--editor-diff-${key.replace(/[A-Z]/g, char => `-${char.toLowerCase()}`)}`] = value;
  }
  return style;
}

/** One paint boundary for native CodeMirror/ProseMirror decorations and React
 * fragments. display:contents preserves each production editor's layout. */
export function EditorDiffHighlight({ appearance, children }: {
  readonly appearance?: EditorDiffHighlightAppearance | undefined;
  readonly children: ReactNode;
}): React.JSX.Element {
  return <div className="editor-diff-highlight" style={appearanceStyle(appearance)}>{children}</div>;
}

/** React renderers use the same roles as the native decoration adapters. */
export function DiffHighlight({ kind, scope = "text", children }: {
  readonly kind: "added" | "removed" | "incoming";
  readonly scope?: "text" | "block" | "atom";
  readonly children: ReactNode;
}): React.JSX.Element {
  const Tag = scope === "block" ? "div" : kind === "removed" ? "del" : kind === "added" ? "ins" : "span";
  return <Tag className={`editor-diff-${kind}-${scope}`}>{children}</Tag>;
}
