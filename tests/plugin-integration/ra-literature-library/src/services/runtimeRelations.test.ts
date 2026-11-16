// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import type {RelationAssertionDraft} from "@research-assistant/core/knowledge";
import type {RuntimeLiteraturePort} from "@research-assistant/workbench-sdk/literature-runtime";
import {createRuntimeLiteraturePort, selectAppRuntime} from "../../../../../../integration/web/src/app/runtime/literature/runtimeLiteratureWorkspace.testSupport.tsx";
import {configureLiteratureServices} from "../../../../../../plugins/ra-literature-library/src/services/configureLiteratureServices";
import {getResearchKnowledgeGateway} from "../../../../../../plugins/ra-literature-library/src/services/knowledgeGateway";
const key = "a".repeat(64), caps = [{id: "literature.concepts", version: 1}];
const node = {kind: "note_page" as const, id: "core-note", label: "Core note", detail: "笔记页面"};
const draft: RelationAssertionDraft = {subject: {kind: "paper", id: "paper"}, object: node, predicateKind: "related_to", scopeNote: " Scope Ω "};
const relation = {id: "relation", subject: {...draft.subject, label: "Paper", detail: ""}, object: node, predicateKind: "related_to" as const, scopeNote: "Scope Ω", provenanceKind: "human_original" as const, epistemicRole: "assertion" as const, revision: 1, evidenceCount: 0, createdAt: "created", updatedAt: "updated"};
const cases = [
  {command: "search_runtime_relation_nodes", args: {query: " Ω\u0000 "}, method: "searchRelationNodes" as const, run: (p: RuntimeLiteraturePort) => p.knowledge.searchRelationNodes!(" Ω\u0000 "), result: [node]},
  {command: "save_runtime_relation_assertion", args: {draft}, method: "saveRelationAssertion" as const, run: (p: RuntimeLiteraturePort) => p.knowledge.saveRelationAssertion!(draft), result: relation},
  {command: "list_runtime_relation_assertions", args: {projectId: " project "}, method: "listRelationAssertions" as const, run: (p: RuntimeLiteraturePort) => p.knowledge.listRelationAssertions!(" project "), result: [relation]},
];
afterEach(() => selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}}));
it.each(cases)("binds $command and preserves arguments, decoding and disposal", async operation => {
  const invoke = vi.fn(async () => operation.result);
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const release = configureLiteratureServices(port);
  try {
    await expect(operation.run(port)).resolves.toEqual(operation.result);
    expect(invoke).toHaveBeenLastCalledWith(operation.command, {packageKey: key, ...operation.args});
    const gateway = getResearchKnowledgeGateway();
    const args = Object.values(operation.args);
    await expect(Reflect.apply(gateway[operation.method], gateway, args)).resolves.toEqual(operation.result);
    release();
    await expect(Reflect.apply(gateway[operation.method], gateway, args)).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  } finally {release();}
});
it.each(cases)("refuses $command without exact capability, optional method or current owner", async operation => {
  const invoke = vi.fn();
  for (const provided of [[], [{id: "literature.concepts", version: 2}]]) await expect(operation.run(createRuntimeLiteraturePort(key, provided, () => undefined, invoke))).rejects.toMatchObject({code: "literature_gateway_unavailable"});
  await expect(operation.run(createRuntimeLiteraturePort(key, caps, () => {throw Object.assign(new Error("closed"), {code: "plugin_activation_context_closed"});}, invoke))).rejects.toMatchObject({code: "plugin_activation_context_closed"});
  expect(invoke).not.toHaveBeenCalled();
  const port = createRuntimeLiteraturePort(key, caps, () => undefined, invoke);
  const partial = {...port, knowledge: {...port.knowledge, [operation.method]: undefined}};
  const release = configureLiteratureServices(partial);
  try {await expect(Reflect.apply(getResearchKnowledgeGateway()[operation.method], undefined, Object.values(operation.args))).rejects.toMatchObject({code: "literature_gateway_unavailable"});} finally {release();}
});
it.each(cases)("rejects malformed $command responses through the original decoder", async operation => {
  const invoke = vi.fn(async () => (operation.method === "saveRelationAssertion" ? {...relation, revision: -1} : [{...node, kind: "unknown"}]));
  const release = configureLiteratureServices(createRuntimeLiteraturePort(key, caps, () => undefined, invoke));
  try {await expect(Reflect.apply(getResearchKnowledgeGateway()[operation.method], undefined, Object.values(operation.args))).rejects.toThrow("不兼容的研究知识数据");expect(invoke).toHaveBeenCalledOnce();} finally {release();}
});
it("keeps relation settlement diagnostics and safe IDs without replay or private data", async () => {
  const cause = {code: "storage_error", message: "Original cause", requestId: "request", instanceId: "instance", relations: {phase: "postcommit-read", outcome: "committed", databaseError: {phase: "read", sqliteExtendedCode: 23}, privateBody: "private body"}};
  const invoke = vi.fn(async () => {throw cause;});
  const error = await createRuntimeLiteraturePort(key, caps, () => undefined, invoke).knowledge.saveRelationAssertion!(draft).catch(e => e);
  expect(error).toMatchObject({relations: cause.relations, requestId: "request", instanceId: "instance", cause});expect(error.message).toContain("relations.outcome=committed");expect(error.message).not.toContain("private body");expect(invoke).toHaveBeenCalledOnce();
});
