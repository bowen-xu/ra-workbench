// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {describe, expect, it, vi} from "vitest";
import type {AgentReferenceInput, AgentSkillCatalog} from "@research-assistant/core/agent";
import type {AgentExtensionFacets} from "../core/contracts/agentExtensions";
import type {BuiltinPluginManifest} from "../core/contracts/plugins";
import {AgentExtensionProvider, coreAgentExtensionFacets} from "../core/extensions/AgentExtensionProvider";
import {AgentResourceList, useAgentResources} from "../core/features/agent/AgentResourceList";
import {AgentReferenceDocument} from "../core/features/agent/AgentReferenceDocument";
import {OPEN_TEXT_DOCUMENT, type TextDocumentRequest} from "../core/components/text-editor/textDocumentEvents";
import {bindBuiltinAgentExtensions, type BuiltinAgentExtensionProvider} from "./builtinAgentExtensions";
import {resolveBuiltinPlugins} from "./pluginComposition";
import {pluginDeclarationFixtures as defaultWorkbench} from "./pluginDeclarations.testSupport";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
const ids = ["core.agent.skills", "core.agent.tools", "core.agent.mcp"] as const;
const nativeToolContract = [{id: "core.agent.tools", version: 1}];
function manifest(id: string, fields: Partial<BuiltinPluginManifest> = {}): BuiltinPluginManifest {
  return {id, apiVersion: 1, dependencies: {required: [], optional: []}, provides: [], consumes: {required: [], optional: []}, ...fields};
}
const provider = (id: string) => manifest(id, {provides: ids.filter(id => id !== "core.agent.tools").map(id => ({id, version: 1}))});
const consumer = (id: string, owner = "source_a") => manifest(id, {consumes: {required: ids.map(id => ({id, version: 1, provider: id === "core.agent.tools" ? "core" : owner})), optional: []}});
function fixtures() {
  const catalog: AgentSkillCatalog = {schemaVersion: 1, environmentId: "application", errors: [], skills: ["first", "second", "disabled"].map(name => ({
    sourceId: `.agents/skills/${name}/SKILL.md`, name, description: "Synthetic instruction", revision: "r1", allowImplicitInvocation: true, enabled: name !== "disabled",
  }))};
  const skills = {provider: "source_a", catalog: {list: vi.fn(async () => catalog)}, readDocument: vi.fn(async () => "Synthetic instruction body; this does not grant tool execution.")};
  const mcp = {provider: "source_a", catalog: {list: vi.fn(async () => ["approved", "unapproved"].map(serverId => ({serverId, environmentId: "fixture", revision: "r1", enabled: true, credentialConfigured: true, approved: serverId === "approved"})))}};
  return {skills, mcp, adapter: {owner: "source_a", skills, mcp} satisfies BuiltinAgentExtensionProvider};
}
function get(facets: ReadonlyMap<string, AgentExtensionFacets>, owner: string) {
  const value = facets.get(owner); if (!value) throw new Error("Fixture consumer missing"); return value;
}

describe("separate Skill / Tool / MCP declarations", () => {
  it.each(ids)("uses the production C01 resolver for %s conflicts, missing capabilities and explicit provider selection", id => {
    const sourceA = manifest("source_a", {provides: [{id, version: 1}]}), sourceB = manifest("source_b", {provides: [{id, version: 1}]});
    const request = manifest("consumer", {consumes: {required: [{id, version: 1}], optional: []}});
    expect(() => resolveBuiltinPlugins([sourceA, sourceB, request], ["source_a", "source_b", "consumer"], [])).toThrow("plugin_capability_ambiguous");
    expect(() => resolveBuiltinPlugins([request], ["consumer"], [])).toThrow("plugin_capability_missing");
    const optional = {...request, consumes: {required: [], optional: [{id, version: 1}]}};
    expect(resolveBuiltinPlugins([optional], ["consumer"], []).unavailableOptional).toEqual([expect.objectContaining({code: "plugin_capability_missing", capabilityId: id})]);
    const selected = {...request, consumes: {required: [{id, version: 1, provider: "source_b"}], optional: []}};
    expect(resolveBuiltinPlugins([sourceA, sourceB, selected], ["source_a", "source_b", "consumer"], []).bindings).toEqual([{pluginId: "consumer", capabilityId: id, provider: "source_b", version: 1}]);
    expect(() => resolveBuiltinPlugins([{...sourceA, provides: [{id, version: 1}, {id, version: 1}]}], ["source_a"], [])).toThrow("plugin_capability_duplicate");
  });

  it("binds all real plugins to the same existing Core providers without exposing management methods", () => {
    const bound = bindBuiltinAgentExtensions(defaultWorkbench.plugins, defaultWorkbench.bindings);
    for (const plugin of defaultWorkbench.plugins) {
      expect(get(bound, plugin.id).owner).toBe(plugin.id);
      expect(get(bound, plugin.id).skills).toBe(coreAgentExtensionFacets.skills);
      expect(get(bound, plugin.id).mcp).toBe(coreAgentExtensionFacets.mcp);
    }
    expect(Object.keys(coreAgentExtensionFacets.skills?.catalog ?? {})).toEqual(["list"]);
    expect(Object.keys(coreAgentExtensionFacets.mcp?.catalog ?? {})).toEqual(["list"]);
  });

  it("fails declared-but-unwired or conflicting adapters and leaves optional absence explicit", () => {
    const defs = [provider("source_a"), consumer("one")], resolved = resolveBuiltinPlugins(defs, ["source_a", "one"], nativeToolContract), data = fixtures();
    expect(() => bindBuiltinAgentExtensions(resolved.plugins, resolved.bindings, [])).toThrow("agent_extension_provider_unavailable");
    expect(() => bindBuiltinAgentExtensions(resolved.plugins, resolved.bindings, [data.adapter, data.adapter])).toThrow("agent_extension_provider_duplicate");
    expect(() => bindBuiltinAgentExtensions(resolved.plugins, resolved.bindings, [{...data.adapter, owner: "another"}])).toThrow("agent_extension_provider_identity_mismatch");
    expect(() => bindBuiltinAgentExtensions([{id: "one"}], [{pluginId: "one", capabilityId: "core.agent.tools", provider: "source_a", version: 1}], [data.adapter])).toThrow("agent_tool_provider_requires_native_binding");
    const empty = manifest("optional", {consumes: {required: [], optional: ids.map(id => ({id, version: 1}))}});
    const absent = resolveBuiltinPlugins([empty], ["optional"], []);
    expect(get(bindBuiltinAgentExtensions(absent.plugins, absent.bindings, []), "optional")).toEqual({owner: "optional"});
  });
});

it("uses real resource controls and reads selected Skill bodies through shared facets while another consumer leaves", async () => {
  const defs = [provider("source_a"), consumer("one"), consumer("two")];
  const resolved = resolveBuiltinPlugins(defs, defs.map(value => value.id), nativeToolContract), data = fixtures();
  const bound = bindBuiltinAgentExtensions(resolved.plugins, resolved.bindings, [data.adapter]);
  expect(get(bound, "one").skills).toBe(get(bound, "two").skills);
  expect(get(bound, "one").mcp).toBe(get(bound, "two").mcp);
  const selected: AgentReferenceInput[] = [];
  function Resources({name, active = true}: {readonly name: string; readonly active?: boolean}) {
    const catalog = useAgentResources(active);
    return <section data-consumer={name}><AgentResourceList {...catalog} onSelect={value => selected.push(value)}/></section>;
  }
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  let opened: TextDocumentRequest | undefined;
  const receive = (event: Event) => {event.preventDefault(); opened = (event as CustomEvent<TextDocumentRequest>).detail;};
  window.addEventListener(OPEN_TEXT_DOCUMENT, receive);
  const render = (one: boolean, active = true, reference?: AgentReferenceInput) => <>
    {one && <AgentExtensionProvider key="one" value={get(bound, "one")}><Resources name="one"/></AgentExtensionProvider>}
    <AgentExtensionProvider key="two" value={get(bound, "two")}><Resources name="two" active={active}/>{reference && <AgentReferenceDocument reference={reference} onClose={() => {}}/>}</AgentExtensionProvider>
  </>;
  try {
    await act(async () => root.render(render(true)));
    for (const name of ["one", "two"]) {
      expect(host.querySelectorAll(`[data-consumer="${name}"] .agent-resource-group-skills button`)).toHaveLength(2);
      expect(host.querySelectorAll(`[data-consumer="${name}"] .agent-resource-group-tools button`)).toHaveLength(2);
      expect(host.querySelector(`[data-consumer="${name}"] .agent-resource-group-tools button:disabled`)?.textContent).toContain("unapproved");
    }
    expect(data.skills.readDocument).not.toHaveBeenCalled();
    await act(async () => host.querySelector<HTMLButtonElement>('[data-consumer="two"] .agent-resource-group-skills button')?.click());
    expect(selected).toEqual([{type: "skill", name: "first", identity: {environment_id: "application", source_id: ".agents/skills/first/SKILL.md", revision: "r1"}}]);
    await act(async () => root.render(render(false, false, selected[0])));
    expect(data.skills.readDocument).toHaveBeenCalledWith(selected[0]?.type === "skill" ? selected[0].identity : undefined, undefined);
    expect(opened?.content).toContain("does not grant tool execution");
    expect(host.querySelector('[data-consumer="one"]')).toBeNull();
    await act(async () => root.render(render(false, true)));
    expect(host.querySelectorAll('[data-consumer="two"] .agent-resource-group-skills button')).toHaveLength(2);
    expect(data.skills.catalog.list).toHaveBeenCalledTimes(3);
    expect(data.mcp.catalog.list).toHaveBeenCalledTimes(3);
    expect(get(bound, "two").mcp).toBe(data.mcp);
  } finally { window.removeEventListener(OPEN_TEXT_DOCUMENT, receive); await act(async () => root.unmount()); host.remove(); }
});

it("keeps optional absence and discovery failures separate and retries through the same provider", async () => {
  const data = fixtures();
  data.skills.catalog.list.mockRejectedValueOnce(new Error("synthetic_discovery_unavailable"));
  const facets: AgentExtensionFacets = {owner: "fixture", skills: data.skills, mcp: data.mcp};
  function Resources({active}: {readonly active: boolean}) {
    const catalog = useAgentResources(active);
    return <AgentResourceList {...catalog} onSelect={() => {}}/>;
  }
  const host = document.createElement("div"), root = createRoot(host); document.body.append(host);
  try {
    await act(async () => root.render(<AgentExtensionProvider value={facets}><Resources active/></AgentExtensionProvider>));
    expect(host.textContent).toContain("部分资源读取失败");
    expect(host.querySelectorAll(".agent-resource-group-skills button")).toHaveLength(0);
    expect(host.querySelectorAll(".agent-resource-group-tools button")).toHaveLength(2);
    await act(async () => root.render(<AgentExtensionProvider value={facets}><Resources active={false}/></AgentExtensionProvider>));
    await act(async () => root.render(<AgentExtensionProvider value={facets}><Resources active/></AgentExtensionProvider>));
    expect(host.textContent).not.toContain("部分资源读取失败");
    expect(host.querySelectorAll(".agent-resource-group-skills button")).toHaveLength(2);
    expect(data.skills.catalog.list).toHaveBeenCalledTimes(2);
    await act(async () => root.render(<AgentExtensionProvider value={{owner: "optional"}}><Resources active/></AgentExtensionProvider>));
    expect(host.textContent).not.toContain("部分资源读取失败");
    expect(host.querySelectorAll(".agent-resource-item")).toHaveLength(0);
  } finally { await act(async () => root.unmount()); host.remove(); }
});
