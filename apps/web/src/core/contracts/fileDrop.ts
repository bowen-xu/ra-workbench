export interface FileDropPosition {
  readonly x: number;
  readonly y: number;
}

/** Window-local file-drop payload; callers own subscription cleanup and domain handling. */
export type WindowFileDropEvent =
  | { readonly type: "leave" }
  | { readonly type: "over"; readonly position: FileDropPosition }
  | { readonly type: "enter" | "drop"; readonly position: FileDropPosition; readonly paths: readonly string[] };
