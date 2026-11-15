export type ShortcutAction = "toggleChat" | "toggleConversations";
export interface ShortcutBinding {
  readonly key: string;
  readonly metaKey: boolean;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
}
export type ShortcutBindings = Readonly<Record<ShortcutAction, ShortcutBinding | null>>;
export const shortcutActions: ReadonlyArray<{ readonly id: ShortcutAction; readonly label: string }> = [
  { id: "toggleChat", label: "打开／关闭聊天窗" },
  { id: "toggleConversations", label: "打开／关闭对话列表" },
];
export const isShortcutMac = (): boolean => /Mac|iPhone|iPad/.test(navigator.platform);
export function defaultShortcuts(mac = isShortcutMac()): Readonly<Record<ShortcutAction, ShortcutBinding>> {
  const modifiers = { metaKey: mac, ctrlKey: !mac, altKey: false, shiftKey: false };
  return { toggleChat: { key: ".", ...modifiers }, toggleConversations: { key: "l", ...modifiers } };
}

const punctuation: Readonly<Record<string, string>> = { Period: ".", Comma: ",", Slash: "/", Backslash: "\\", Semicolon: ";", Quote: "'", BracketLeft: "[", BracketRight: "]", Minus: "-", Equal: "=", Backquote: "`" };
const supportedKey = (key: string): boolean => /^[a-z0-9]$/.test(key) || Object.values(punctuation).includes(key)
  || /^(F([1-9]|1[0-2])|Arrow(Up|Down|Left|Right)|Home|End|PageUp|PageDown|Enter|Backspace|Delete|Space)$/.test(key);
export function bindingFromEvent(event: Pick<KeyboardEvent, "key" | "code" | "metaKey" | "ctrlKey" | "altKey" | "shiftKey">): ShortcutBinding | undefined {
  const key = punctuation[event.code] ?? (/^Digit[0-9]$/.test(event.code) ? event.code.slice(5) : event.altKey && /^Key[A-Z]$/.test(event.code) ? event.code.slice(3).toLowerCase() : event.key.length === 1 ? event.key.toLowerCase() : event.key);
  const normalized = key === " " ? "Space" : key;
  if (!supportedKey(normalized)) return undefined;
  return { key: normalized, metaKey: event.metaKey, ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey };
}
export function sameShortcut(a: ShortcutBinding | null, b: ShortcutBinding | null): boolean {
  if (a === null || b === null) return a === b;
  return a.key === b.key && a.metaKey === b.metaKey && a.ctrlKey === b.ctrlKey && a.altKey === b.altKey && a.shiftKey === b.shiftKey;
}
export function matchesShortcut(event: KeyboardEvent, binding: ShortcutBinding | null): boolean {
  if (binding === null) return false;
  const pressed = bindingFromEvent(event);
  return pressed !== undefined && sameShortcut(pressed, binding);
}
export function shortcutProblem(binding: ShortcutBinding, mac = isShortcutMac()): string | undefined {
  if (!binding.metaKey && !binding.ctrlKey && !binding.altKey) return "请同时按住 Command、Control 或 Alt／Option 等修饰键。";
  const primary = mac ? binding.metaKey && !binding.ctrlKey : binding.ctrlKey && !binding.metaKey;
  if (primary && !binding.altKey && ((!binding.shiftKey && ["a", "b", "c", "d", "f", "h", "i", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "Enter", "Space", "[", "]"].includes(binding.key))
    || (binding.shiftKey && ["a", "b", "c", "d", "f", "l", "m", "n", "p", "s", "t", "w", "z", "Space"].includes(binding.key)))) return "这个组合已用于常用系统、编辑或应用操作，请换一个组合。";
  if (mac && binding.metaKey && binding.ctrlKey && !binding.altKey && !binding.shiftKey && ["ArrowDown", "ArrowLeft", "ArrowRight", "Space"].includes(binding.key)) return "这个组合已用于布局切换或系统操作，请换一个组合。";
  return undefined;
}
export function formatShortcut(binding: ShortcutBinding | null, mac = isShortcutMac()): string {
  if (binding === null) return "";
  const key = ({ ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→", Space: "Space" } as Record<string, string>)[binding.key] ?? (binding.key.length === 1 ? binding.key.toUpperCase() : binding.key);
  return [binding.ctrlKey ? (mac ? "⌃" : "Ctrl") : "", binding.altKey ? (mac ? "⌥" : "Alt") : "", binding.shiftKey ? (mac ? "⇧" : "Shift") : "", binding.metaKey ? (mac ? "⌘" : "Meta") : "", key].filter(Boolean).join(mac ? " " : " + ");
}
function validBinding(value: unknown): value is ShortcutBinding {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return typeof record.key === "string" && supportedKey(record.key) && ["metaKey", "ctrlKey", "altKey", "shiftKey"].every(key => typeof record[key] === "boolean");
}
export function readShortcuts(value: unknown, mac = isShortcutMac()): ShortcutBindings {
  const defaults = defaultShortcuts(mac);
  if (!value || typeof value !== "object") return defaults;
  const record = value as Record<string, unknown>;
  const chat = record.toggleChat === null ? null : validBinding(record.toggleChat) && !shortcutProblem(record.toggleChat, mac) ? record.toggleChat : defaults.toggleChat;
  const conversations = record.toggleConversations === null ? null : validBinding(record.toggleConversations) && !shortcutProblem(record.toggleConversations, mac) ? record.toggleConversations : defaults.toggleConversations;
  return chat !== null && conversations !== null && sameShortcut(chat, conversations) ? defaults : { toggleChat: chat, toggleConversations: conversations };
}
