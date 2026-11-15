import {expect,it} from "vitest";
import {EMPTY_AGENT_CONTEXT,selectAgentContext,type AgentContextSelection} from "@research-assistant/core/agent";
import {selectedCatalogItem} from "./contextCatalogSelection";

it("does not upgrade an old standalone reference when projecting a new writable group catalog",()=>{
  const item:AgentContextSelection={label:"Note",resource:{kind:"noteGroup",resourceId:"note",groupAccess:"readWrite",newMemberAccess:"readWrite",parts:[
    {resource:{kind:"note",resourceId:"note"},label:"正文",selected:true,access:"readWrite"},
  ]}};
  const saved=selectAgentContext(EMPTY_AGENT_CONTEXT,{label:"Note",resource:{kind:"note",resourceId:"note"}},"persistent");
  expect(selectedCatalogItem(item,saved)).toEqual({label:"Note",resource:{kind:"noteGroup",resourceId:"note",parts:[
    {resource:{kind:"note",resourceId:"note"},label:"正文",selected:true,access:"readOnly"},
  ]}});
});

it("projects an exact template grant alongside the existing paper group without upgrading it", () => {
  const item: AgentContextSelection = {label: "Paper", resource: {kind: "paperGroup", resourceId: "paper", parts: [
    {label: "Reading", resource: {kind: "structuredObject", resourceId: "reading"}, selected: true, access: "readWrite"},
    {label: "Template", resource: {kind: "structuredObject", resourceId: "template"}, selected: true, access: "readWrite"},
  ]}};
  if (item.resource.kind !== "paperGroup") throw new Error("fixture");
  const state = {...EMPTY_AGENT_CONTEXT, persistent: [
    {label: "Paper", resource: {...item.resource, parts: item.resource.parts.slice(0, 1)}},
    {label: "Template", resource: {kind: "structuredObject" as const, resourceId: "template", access: "readOnly" as const}},
  ]};
  const selected = selectedCatalogItem(item, state);
  expect(selected).toMatchObject({resource: {parts: [
    {selected: true, access: "readWrite"}, {selected: true, access: "readOnly"},
  ]}});
  expect(state.persistent).toHaveLength(2);
});
