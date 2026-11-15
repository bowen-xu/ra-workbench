export interface PreviewTab { readonly id: string; readonly title: string; readonly preview: boolean }
/** Only one replaceable preview per workspace. Existing pinned tabs never demote. */
export function openPreviewTab(tabs: readonly PreviewTab[], id: string, title: string, pinned = false): readonly PreviewTab[] {
  if (tabs.some(tab => tab.id === id)) return tabs.map(tab => tab.id === id ? { ...tab, title, preview: tab.preview && !pinned } : tab);
  const next = { id, title, preview: !pinned };
  const index = !pinned ? tabs.findIndex(tab => tab.preview) : -1;
  return index < 0 ? [...tabs, next] : tabs.map((tab, i) => i === index ? next : tab);
}
