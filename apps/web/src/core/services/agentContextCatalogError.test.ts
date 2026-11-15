import { expect, it } from "vitest";
import { contextCatalogErrorMessage } from "./agentContextCatalogError";

const failure = {
  code: "plugin_data_sql_failed",
  message: "文献上下文目录来源不可用。",
  packageKey: "a".repeat(64),
  phase: "read",
  resourceId: "literature.context-catalog.papers",
  resourceVersion: 1,
  safeCause: "sqlite_failure",
  sqliteCode: "DatabaseBusy",
  sqliteExtendedCode: 5,
};

it("keeps the finite package, resource and SQLite diagnostic without a host identity", () => {
  const message = contextCatalogErrorMessage(failure);
  for (const [key, value] of Object.entries(failure).filter(([key]) => key !== "message")) {
    expect(message).toContain(`${key}=${value}`);
  }
  expect(message).not.toMatch(/(?:requestId|instanceId)=/u);
});

it("keeps an admission failure without inventing a resource or request", () => {
  const message = contextCatalogErrorMessage({
    code: "plugin_package_missing", message: failure.message, packageKey: failure.packageKey,
    phase: "admission", safeCause: "package_missing",
  });
  expect(message).toContain("code=plugin_package_missing");
  expect(message).toContain(`packageKey=${failure.packageKey}`);
  expect(message).not.toMatch(/(?:resourceId|resourceVersion|requestId|instanceId)=/u);
});

it("retains the actual resource-admission code", () => {
  const message = contextCatalogErrorMessage({
    ...failure, code: "runtime_plugin_data_digest_mismatch", phase: "resource",
    safeCause: "runtime_plugin_data_digest_mismatch",
  });
  expect(message).toContain("code=runtime_plugin_data_digest_mismatch");
  expect(message).toContain("resourceId=literature.context-catalog.papers");
});

it("does not display arbitrary payloads, paths, SQL or malformed identities", () => {
  const message = contextCatalogErrorMessage({
    ...failure, packageKey: "/private/catalog.sqlite3", resourceId: "private-token",
    safeCause: "select private_text from notes", sqliteCode: "Authorization: secret",
    sqliteExtendedCode: 5.5, requestId: "invented-request", instanceId: "invented-host",
    rawResponse: "private-body", sql: "select secret", path: "/private/file.pdf",
  });
  expect(message).toContain("code=plugin_data_sql_failed");
  expect(message).not.toMatch(/private|secret|select|invented|resourceVersion/u);
});

it.each([undefined, null, [], "unavailable", new Error("private body"),
  { ...failure, message: "/private/database.sqlite3" },
  { ...failure, phase: "execute-arbitrary-sql" },
  { ...failure, code: "private-body" },
])("preserves the existing generic message for legacy or unrecognized failures: %j", reason => {
  expect(contextCatalogErrorMessage(reason)).toBe("无法读取材料，请重试。");
});
