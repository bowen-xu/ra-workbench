/** Identical controls for CodeMirror and ProseMirror decoration widgets. */
export function createEditorDiffChunkActions({ document, number, disabled = false, onResolve, conflict, labels }: {
  readonly document: Document;
  readonly number: number;
  readonly disabled?: boolean;
  readonly onResolve: (action: "keep" | "undo") => void;
  readonly labels?: { readonly undo?: string; readonly keep?: string };
  readonly conflict?: { readonly choice: string | undefined; readonly onBoth: () => void };
}): HTMLElement {
  const row = document.createElement("div");
  row.className = "editor-diff-chunk-actions writing-diff-chunk-actions";
  row.setAttribute("role", "group");
  const group = row.appendChild(document.createElement("div"));
  for (const action of ["undo", "keep"] as const) {
    const button = group.appendChild(document.createElement("button"));
    button.type = "button";
    button.className = `writing-diff-chunk-${action}`;
    button.textContent = labels?.[action] ?? (conflict ? action === "keep" ? "保留当前编辑" : "保留已保存内容" : action === "keep" ? "Keep" : "Undo");
    if (conflict) {
      button.dataset.conflictLabel = button.textContent;
      button.setAttribute("aria-pressed", String(conflict.choice === (action === "keep" ? "current" : "saved")));
    }
    button.addEventListener("pointerdown", event => event.preventDefault());
    button.addEventListener("click", () => { if (!button.disabled) onResolve(action); });
  }
  if (conflict) {
    const button = group.appendChild(document.createElement("button"));
    button.type = "button";
    button.textContent = "两者都保留";
    button.dataset.conflictLabel = button.textContent;
    button.setAttribute("aria-pressed", String(conflict.choice === "both"));
    button.addEventListener("pointerdown", event => event.preventDefault());
    button.addEventListener("click", () => { if (!button.disabled) conflict.onBoth(); });
  }
  updateEditorDiffChunkActions(row, { number, disabled });
  return row;
}

/** Update review state without replacing focused/hovered decoration controls. */
export function updateEditorDiffChunkActions(row: HTMLElement, { number, disabled }: {
  readonly number: number; readonly disabled: boolean;
}): void {
  const label = `第 ${number} 处变更操作`;
  if (row.getAttribute("aria-label") !== label) row.setAttribute("aria-label", label);
  row.querySelectorAll<HTMLButtonElement>("button").forEach(button => {
    const keep = button.classList.contains("writing-diff-chunk-keep");
    const label = button.dataset.conflictLabel ? `第 ${number} 处差异：${button.dataset.conflictLabel}` : `${keep ? "保留" : "撤销"}第 ${number} 处变更`;
    if (button.getAttribute("aria-label") !== label) button.setAttribute("aria-label", label);
    if (button.disabled !== disabled) button.disabled = disabled;
  });
}
