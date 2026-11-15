import { createElement, isValidElement, type MouseEvent } from "react";
import { toast, type ExternalToast } from "sonner";
import { NotificationActions, type NotificationAction } from "./NotificationActions";
export type { NotificationAction } from "./NotificationActions";

export type NotificationKind = "error" | "info" | "loading" | "success" | "warning";
export interface NotificationOptions extends ExternalToast {
  readonly actions?: readonly NotificationAction[];
}
export interface NotificationUpdate extends NotificationOptions {
  readonly message?: string;
}
interface Entry {
  readonly id: string | number;
  readonly presentationId: string | number;
  readonly kind: NotificationKind;
  readonly message: string;
  readonly options: NotificationOptions;
  readonly protected: boolean;
  readonly hidden: boolean;
}
const entries = new Map<string | number, Entry>();
const listeners = new Set<() => void>();
const pendingActions = new Map<string | number, string>();
let hidden: readonly Entry[] = [];
let sequence = 0;
function publish(): void {
  hidden = [...entries.values()].filter(entry => entry.hidden);
  for (const listener of listeners) listener();
}
export function subscribeHiddenNotifications(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}
export function getHiddenNotifications(): readonly { readonly id: string | number; readonly message: string }[] {
  return hidden;
}
/** A task may publish while the application root is still mounting. */
export function replayVisibleNotifications(): void {
  for (const entry of entries.values()) if (!entry.hidden) render(entry);
}
function render(entry: Entry): void {
  const { actions, ...options } = entry.options;
  let action = options.action;
  let cancel = options.cancel;
  const legacy = [action, cancel].flatMap((value, index) => value && !isValidElement(value) && typeof value === "object"
    && "onClick" in value && typeof value.onClick === "function" && typeof value.label === "string"
    ? [{id: index === 0 ? "legacy-action" : "legacy-cancel", label: value.label, onClick: value.onClick}] : []);
  if (actions !== undefined || legacy.length) {
    const visible: readonly NotificationAction[] = actions ?? legacy.map(item => ({id: item.id, label: item.label, onClick: () => undefined}));
    action = createElement(NotificationActions, { actions: visible, pendingActionId: pendingActions.get(entry.id),
      onRun: async (selected, event?: MouseEvent<HTMLButtonElement>) => {
        if (pendingActions.has(entry.id) || selected.disabled) return;
        const old = actions === undefined ? legacy.find(item => item.id === selected.id) : undefined;
        if (old && event) {
          if (entry.protected) event.preventDefault();
          old.onClick(event);
          if (!event.defaultPrevented && entries.get(entry.id) === entry) notify.dismiss(entry.id);
          return;
        }
        event?.preventDefault();
        pendingActions.set(entry.id, selected.id); notify.update(entry.id, {});
        try { await selected.onClick(); }
        catch (reason) { notify.error(reason instanceof Error ? reason.message : "操作失败，请重试"); }
        finally { pendingActions.delete(entry.id); notify.update(entry.id, {}); }
      } });
    if (actions === undefined) cancel = null;
  }
  const dismiss = (value: Parameters<NonNullable<ExternalToast["onDismiss"]>>[0]) => {
    if (entries.get(entry.id) !== entry) return; // Ignore late callbacks from an earlier presentation.
    if (entry.protected) { entries.set(entry.id, { ...entry, hidden: true }); }
    else { entries.delete(entry.id); options.onDismiss?.({ ...value, id: entry.id }); }
    publish();
  };
  const common: ExternalToast = {
    ...options, id: entry.presentationId, action: action ?? null, cancel: cancel ?? null,
    dismissible: true, closeButton: entry.protected ? false : options.closeButton ?? true,
    duration: entry.protected ? Infinity : options.duration ?? 4_500,
    onDismiss: dismiss,
    onAutoClose: value => {
      if (entries.get(entry.id) !== entry) return;
      entries.delete(entry.id); publish(); options.onAutoClose?.({ ...value, id: entry.id });
    },
  };
  // Sonner disables swiping for type=loading. Keep its native swipe/stack renderer,
  // but represent loading with an icon so a swipe can hide the logical task.
  if (entry.kind === "loading") {
    toast(entry.message, { ...common, icon: createElement("span", { className: "app-notification-spinner", "aria-hidden": true }) });
  } else {
    toast[entry.kind](entry.message, { ...common, icon: options.icon ?? undefined });
  }
}
function show(kind: NotificationKind, message: string, options: NotificationOptions = {}): string | number {
  const id = options.id ?? `notification:${++sequence}`;
  const previous = entries.get(id);
  const protectedMessage = kind === "loading" || options.dismissible === false;
  const duration = kind === "error" ? 7_000 : kind === "warning" ? 6_000 : undefined;
  const entry: Entry = {
    id, presentationId: previous?.hidden && !protectedMessage ? `notification-view:${++sequence}` : previous?.presentationId ?? id,
    kind, message, options: { ...(duration === undefined ? {} : { duration }), ...options }, protected: protectedMessage,
    hidden: protectedMessage && previous?.hidden === true,
  };
  entries.set(id, entry); publish();
  if (!entry.hidden) render(entry);
  return id;
}
export const notify = {
  dismiss(id: string | number): void {
    const entry = entries.get(id); entries.delete(id); publish();
    toast.dismiss(entry?.presentationId ?? id);
  },
  hide(id: string | number): void {
    const entry = entries.get(id);
    if (!entry || entry.hidden) return;
    entries.set(id, { ...entry, hidden: true }); publish(); toast.dismiss(entry.presentationId);
  },
  showHidden(): void {
    for (const previous of hidden) {
      const entry = { ...previous, hidden: false, presentationId: `notification-view:${++sequence}` };
      entries.set(entry.id, entry); render(entry);
    }
    publish();
  },
  update(id: string | number, update: NotificationUpdate): void {
    const entry = entries.get(id);
    if (!entry) return;
    const { message, ...options } = update;
    const next = { ...entry, message: message ?? entry.message, options: { ...entry.options, ...options } };
    entries.set(id, next); publish(); if (!next.hidden) render(next);
  },
  error: (message: string, options?: NotificationOptions) => show("error", message, options),
  info: (message: string, options?: NotificationOptions) => show("info", message, options),
  loading: (message: string, options?: NotificationOptions) => show("loading", message, options),
  success: (message: string, options?: NotificationOptions) => show("success", message, options),
  warning: (message: string, options?: NotificationOptions) => show("warning", message, options),
};
