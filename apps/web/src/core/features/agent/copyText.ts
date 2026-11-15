/** Copy text in both secure browser contexts and the desktop webview. */
export async function copyText(value: string, formats?: Readonly<Record<string, string>>): Promise<boolean> {
  if (!value) return false;
  try {
    if (formats && navigator.clipboard?.write && typeof ClipboardItem !== "undefined") {
      await navigator.clipboard.write([new ClipboardItem(Object.fromEntries(Object.entries({"text/plain": value, ...formats})
        .map(([type, text]) => [type, new Blob([text], {type})])))]);
      return true;
    }
    if (!formats && navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value);
      return true;
    }
  } catch {
    // Fall through to the textarea path for restricted webviews.
  }
  const textarea = document.createElement("textarea");
  textarea.value = value;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.opacity = "0";
  const active = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  const selection = document.getSelection(), ranges = selection ? Array.from({length: selection.rangeCount}, (_, index) => selection.getRangeAt(index).cloneRange()) : [];
  document.body.appendChild(textarea);
  textarea.select();
  let copied = false;
  let written = !formats;
  const write = (event: ClipboardEvent) => {
    if (!formats || !event.clipboardData) return;
    event.preventDefault();
    try {
      for (const [type, text] of Object.entries({"text/plain": value, ...formats})) event.clipboardData.setData(type, text);
      written = true;
    } catch { written = false; }
  };
  if (formats) document.addEventListener("copy", write, true);
  try {
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  } finally {
    document.removeEventListener("copy", write, true);
    textarea.remove();
    active?.focus({preventScroll: true});
    if (selection) { selection.removeAllRanges(); for (const range of ranges) if (range.commonAncestorContainer.isConnected) selection.addRange(range); }
  }
  return copied && written;
}
