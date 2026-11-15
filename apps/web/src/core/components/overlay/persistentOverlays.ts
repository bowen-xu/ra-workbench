const notifications = new Set<HTMLElement>();

export function registerNotificationOverlay(root: HTMLElement): () => void {
  notifications.add(root);
  return () => { notifications.delete(root); };
}

/** Notifications remain actionable while an Ariakit modal isolates its page. */
export function persistentOverlays(doc: Document): HTMLElement[] {
  return [...notifications].filter(root => root.ownerDocument === doc && root.isConnected);
}
