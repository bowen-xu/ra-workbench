/** Left mouse button glyph for shortcut hints; accessible wording is not visible text. */
export function MouseLeftButtonIcon(): React.JSX.Element {
  return <svg viewBox="0 0 16 20" width="12" height="15" role="img" aria-label="鼠标左键" focusable="false">
    <path d="M8 1C4.1 1 1 4.1 1 8v4c0 3.9 3.1 7 7 7s7-3.1 7-7V8c0-3.9-3.1-7-7-7Z" fill="none" stroke="currentColor" strokeWidth="1.5" />
    <path d="M7 2.1C4.2 2.6 2 5 2 8v1h5V2.1Z" fill="currentColor" />
    <path d="M8 1v8M1 9h14" fill="none" stroke="currentColor" strokeWidth="1" />
  </svg>;
}

export function MouseLeftShortcut({ modifier }: { readonly modifier: "⌘" | "⌥" }): React.JSX.Element {
  return <span style={{ display: "inline-flex", alignItems: "center", gap: 3 }}>{modifier} + <MouseLeftButtonIcon /></span>;
}
