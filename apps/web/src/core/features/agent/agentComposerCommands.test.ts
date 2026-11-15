import { expect, it } from "vitest";
import { composerCommandQuery, readComposerCommand } from "./agentComposerCommands";

it.each(["/compact", " /compact  "])("recognizes the whole-input command %s", text => {
  expect(readComposerCommand(text)).toEqual({ name: "compact", arguments: "" });
});
it.each(["@compact", "说明 /compact", "/Users/fixture/main.tex", "https://example.test/path", "/compact\n其他正文"])("keeps prose and paths ordinary: %s", text => {
  expect(readComposerCommand(text)).toBeNull();
  expect(composerCommandQuery(text, text.length)).toBeNull();
});
it("preserves arguments and distinguishes unknown commands for UI validation", () => {
  expect(readComposerCommand("/compact 参数")).toEqual({ name: "compact", arguments: "参数" });
  expect(readComposerCommand("/unknown")).toEqual({ name: "unknown", arguments: "" });
  expect(composerCommandQuery("/co", 3)).toBe("co");
  expect(composerCommandQuery("/compact", 0)).toBeNull();
});
