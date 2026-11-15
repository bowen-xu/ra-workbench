import { expect, it } from "vitest";
import { appendMcpServer, type McpServerDraft } from "./mcpServerDraft";

const draft: McpServerDraft = { name: "fixture", transport: "stdio", program: "/usr/bin/node", directory: "/tmp/fixture", url: "", arguments: ["server.js", 'a"b'], environment: [{ key: "MODE", value: "test" }], credentialHandle: "mcp:fixture" };
it("appends a bounded explicit table without changing any original configuration", () => {
  const original = 'schema_version = 1\n# retain bytes\n';
  const next = appendMcpServer(original, draft);
  expect(next.startsWith(original)).toBe(true);
  expect(next).toContain('[servers."fixture"]'); expect(next).toContain('arguments = ["server.js", "a\\"b"]');
  expect(next).toContain('environment = { "MODE" = "test" }'); expect(next).toContain('credential_handle = "mcp:fixture"');
});
it("rejects invalid paths, controls, duplicated environment keys and unsafe HTTP URLs", () => {
  expect(() => appendMcpServer("", { ...draft, program: "node" })).toThrow("绝对路径");
  expect(() => appendMcpServer("", { ...draft, name: "bad\nname" })).toThrow();
  expect(() => appendMcpServer("", { ...draft, environment: [...draft.environment, ...draft.environment] })).toThrow("不能重复");
  expect(() => appendMcpServer("", { ...draft, transport: "streamable_http", url: "http://example.com/mcp" })).toThrow();
  expect(() => appendMcpServer("", { ...draft, transport: "streamable_http", url: "https://user:secret@example.com/mcp" })).toThrow();
  const http = appendMcpServer("", { ...draft, transport: "streamable_http", url: "http://127.0.0.1:3000/mcp" });
  expect(http).toContain('transport = "streamable_http"'); expect(http).not.toContain('program ='); expect(http).not.toContain('environment =');
});
