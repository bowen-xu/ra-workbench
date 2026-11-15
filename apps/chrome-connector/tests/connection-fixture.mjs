// SPDX-License-Identifier: AGPL-3.0-or-later
import fs from "node:fs/promises";
import vm from "node:vm";
import { webcrypto } from "node:crypto";
import ts from "typescript";

const context = { ArrayBuffer, Uint8Array, URLSearchParams, AbortSignal, Error, TextEncoder, TextDecoder, crypto: webcrypto };
vm.createContext(context);
for (const name of ["connection", "transport"]) {
  const source = await fs.readFile(new URL(`../src/${name}.ts`, import.meta.url), "utf8");
  vm.runInContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.None } }).outputText, context);
}
export const { createTransport, STORAGE_KEY } = context.ResearchAssistant;
export const descriptor = overrides => ({ application: "research-assistant", version: "0.2.0",
  connectorApiVersion: 2, zoteroConnectorApiVersion: 3, environment: "development",
  receiverId: "a".repeat(64), instanceId: "b".repeat(32), connectionToken: "c".repeat(64), prefs: {}, ...overrides });
export const response = (value, overrides = {}) => new Response(JSON.stringify(value), {
  status: overrides.status ?? 200, headers: { "Content-Type": "application/json",
    "X-Research-Assistant-Application": "research-assistant", "X-Research-Assistant-Version": "0.2.0",
    "X-Research-Assistant-Connector-API-Version": "2", "X-Zotero-Connector-API-Version": "3",
    "X-Research-Assistant-Instance-ID": "b".repeat(32), ...overrides.headers },
});
export function fixture(fetcher, initial = { schemaVersion: 2, environment: "development" }) {
  const values = initial === undefined ? {} : { [STORAGE_KEY]: initial };
  const storage = { get: async () => structuredClone(values), set: async input => { Object.assign(values, structuredClone(input)); } };
  return { values, storage, request: createTransport(storage, fetcher, "0.1.0") };
}
