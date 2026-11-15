import {expect, it, vi} from "vitest";
import type {AgentContextResource, AgentUserInput} from "@research-assistant/core/agent";
import {contextComposerResources, turnContextResources} from "./contextComposerResources";
import type {AgentContextGateway} from "../../../services/agentContextGateway";
import {createMockAgentResourceGateways} from "../../../../app/previews/agent/mockAgentResourceGateways";

function mention(resource: AgentContextResource): AgentUserInput {
  return {type: "mention", name: "Same display name", target: {kind: "file",
    sourceId: `context:${JSON.stringify(resource)}`, resourceId: resource.resourceId, revision: "current"}};
}
it("deduplicates exact temporary references while keeping different attachments separate", () => {
  const first = {kind: "paper" as const, resourceId: "paper", attachmentId: "first"};
  const second = {...first, attachmentId: "second"};
  expect(turnContextResources([mention(first), mention(first), mention(second)])).toEqual([first, second]);
});
it("rejects malformed resource bindings instead of guessing from display names", () => {
  expect(() => turnContextResources([mention({kind: "manuscriptFile", resourceId: "m", path: "../private.tex"})])).toThrow();
});
it("keeps opened materials separate from full-library queries and retains exact identities", async () => {
  const opened = [
    {label: "Open paper", resource: {kind: "paper" as const, resourceId: "paper", attachmentId: "attachment"}},
    {label: "Open note", resource: {kind: "note" as const, resourceId: "note"}},
    {label: "Project · main.tex", resource: {kind: "manuscriptFile" as const, resourceId: "project", path: "main.tex"}},
  ];
  const closed = {label: "Closed note", resource: {kind: "note" as const, resourceId: "closed-note"}};
  const search = vi.fn(async () => [closed]);
  const gateways = contextComposerResources({search} as unknown as AgentContextGateway, undefined, opened);
  expect(gateways.openedResources?.map(item => item.label)).toEqual(opened.map(item => item.label));
  expect(gateways.openedResources?.every(item => item.source === "")).toBe(true);
  expect(search).not.toHaveBeenCalled();
  expect(turnContextResources(gateways.openedResources!.map(item => item.input))).toEqual(opened.map(item => item.resource));
  const results = await gateways.conversationResources!("Closed");
  expect(search).toHaveBeenCalledExactlyOnceWith("Closed");
  expect(results[0]?.input.name).toBe("Closed note");
  expect(results[0]?.source).toBe("");
  expect(turnContextResources(results.map(item => item.input))).toEqual([closed.resource]);
});
it("uses an injected file catalog without falling through to the real context gateway", async () => {
  const fullSearch = vi.fn(async () => { throw new Error("real gateway must not run"); });
  const result = { group: "files" as const, label: "main.tex", source: "当前稿件", input: {
    type: "mention" as const, name: "main.tex", target: { kind: "file" as const, sourceId: "writing:mock", resourceId: "main.tex", revision: "v1" },
  } };
  const mockSearch = vi.fn(async () => [result]);
  const resources = contextComposerResources({ search: fullSearch } as unknown as AgentContextGateway,
    { ...createMockAgentResourceGateways(), conversationResources: mockSearch });
  expect(await resources.conversationResources!("main")).toEqual([result]);
  expect(mockSearch).toHaveBeenCalledExactlyOnceWith("main");
  expect(fullSearch).not.toHaveBeenCalled();
});

it("refreshes an open paper's ordinary notes when reopening @ without loading note bodies", async () => {
  const pdf={resource:{kind:"paper" as const,resourceId:"paper",attachmentId:"pdf"},label:"paper.pdf",selected:true,access:"readOnly" as const};
  const note={resource:{kind:"note" as const,resourceId:"note"},label:"算法整理",selected:false,access:"readOnly" as const};
  const opened={label:"Open paper",resource:{kind:"paperGroup" as const,resourceId:"paper",newMemberAccess:"readOnly" as const,parts:[pdf]}};
  const catalog=vi.fn(async()=>[{...opened,resource:{...opened.resource,parts:[pdf,note]}}]);
  const resources=contextComposerResources({catalog} as unknown as AgentContextGateway,undefined,[opened]);
  expect(resources.openedResources).toHaveLength(1);
  const refreshed=await resources.refreshOpenedResources!();
  expect(catalog).toHaveBeenCalledExactlyOnceWith("Open paper");
  expect(refreshed.map(row=>row.label)).toEqual(["Open paper · paper.pdf","算法整理"]);
  expect(refreshed[0]?.source).toBe("");
  expect(refreshed[1]?.source).toBe("Open paper");
  expect(refreshed[1]?.input.name).toBe("算法整理");
  expect(turnContextResources(refreshed.map(row=>row.input))).toEqual([pdf.resource,note.resource]);
});
