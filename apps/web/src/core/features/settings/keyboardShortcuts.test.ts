// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { bindingFromEvent, defaultShortcuts, formatShortcut, matchesShortcut, readShortcuts, shortcutProblem } from "./keyboardShortcuts";

describe("application shortcut bindings", () => {
  it("uses the requested defaults with exact modifiers on each platform", () => {
    const mac = defaultShortcuts(true), windows = defaultShortcuts(false);
    expect(matchesShortcut(new KeyboardEvent("keydown", { key: ".", metaKey: true }), mac.toggleChat)).toBe(true);
    expect(matchesShortcut(new KeyboardEvent("keydown", { key: "l", metaKey: true }), mac.toggleConversations)).toBe(true);
    expect(matchesShortcut(new KeyboardEvent("keydown", { key: "l", ctrlKey: true }), windows.toggleConversations)).toBe(true);
    expect(matchesShortcut(new KeyboardEvent("keydown", { key: "l", metaKey: true, shiftKey: true }), mac.toggleConversations)).toBe(false);
    expect(matchesShortcut(new KeyboardEvent("keydown", { key: "l", metaKey: true, altKey: true }), mac.toggleConversations)).toBe(false);
    expect(formatShortcut(mac.toggleChat, true)).toBe("⌘ .");
    expect(formatShortcut(windows.toggleConversations, false)).toBe("Ctrl + L");
  });
  it("ignores modifier-only keys and normalizes shifted punctuation and Option letters", () => {
    expect(bindingFromEvent(new KeyboardEvent("keydown", { key: "Meta", metaKey: true }))).toBeUndefined();
    expect(bindingFromEvent(new KeyboardEvent("keydown", { key: ">", code: "Period", metaKey: true, shiftKey: true }))?.key).toBe(".");
    expect(bindingFromEvent(new KeyboardEvent("keydown", { key: "˚", code: "KeyK", altKey: true, metaKey: true }))?.key).toBe("k");
  });
  it("preserves standard commands and rejects corrupt or duplicate stored bindings", () => {
    const defaults = defaultShortcuts(true);
    expect(shortcutProblem({ ...defaults.toggleChat, key: "q" }, true)).toBeDefined();
    expect(shortcutProblem({ ...defaults.toggleChat, metaKey: false }, true)).toBeDefined();
    expect(readShortcuts({ toggleChat: defaults.toggleChat, toggleConversations: defaults.toggleChat }, true)).toEqual(defaults);
    expect(readShortcuts({ toggleChat: { key: "q" }, toggleConversations: "invalid" }, true)).toEqual(defaults);
    const custom = { ...defaults, toggleChat: { ...defaults.toggleChat, key: "k", shiftKey: true } };
    expect(readShortcuts(custom, true)).toEqual(custom);
  });
});
