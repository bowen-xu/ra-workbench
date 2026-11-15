// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { decodeAgentUserInput, type AgentContextText } from "@research-assistant/core/agent";
import type { PaperOverviewUnitCandidateV1 } from "@research-assistant/core/ai";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { mockInternalTaskInput } from "./mockInternalTaskInput";
import { readMockBundledTaskSkill } from "./mockBundledTaskSkills";

describe("internal task composer references", () => {
  const target: StructuredResource = {id:"reading",kind:"structuredReading",paperId:"paper",sourceUri:"source://library/paper/reading",
    title:"结构化阅读", revision:0,content:null,reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[]};
  const context: AgentContextText[] = [
    {sourceId:"pdf-grant",resourceId:"paper",revision:"grant:pdf",kind:"paper",anchor:JSON.stringify({resource:{kind:"paper",resourceId:"paper",attachmentId:"pdf"}}),text:JSON.stringify({label:"论文 🧪.pdf"})},
    {sourceId:"target-grant",resourceId:"reading",revision:"grant:reading",kind:"paper",anchor:"{}",text:JSON.stringify({label:"结构化阅读"})},
  ];
  const payload: PaperOverviewUnitCandidateV1 = {kind:"paper_overview_unit_v1",schemaVersion:1,profileId:"generic_overview_v1",
    fields:[{field:"research_question",status:"not_found",statements:[]}]};

  it("persists bound files and a readable, versioned skill rather than raw task or source data", async () => {
    const input = await mockInternalTaskInput(context, target, payload, localStorage);
    expect(input.text).toContain("[论文 🧪.pdf](source://library/paper/pdf)");
    expect(input.text).toContain("[结构化阅读](source://library/paper/reading)");
    expect(input.text).toContain("Update File");
    expect(input.text).not.toContain("task_prompt");
    expect(input.text).not.toContain("quoteExact");
    const decoded = decodeAgentUserInput(input.userInput, input.text);
    expect(decoded).toEqual(input.userInput);
    const skill = decoded.find(value => value.type === "skill");
    if (skill?.type !== "skill") throw new Error("missing selected skill");
    expect(await readMockBundledTaskSkill(skill.identity)).toContain("Update File");
    const text = decoded[0];
    if (text?.type !== "text") throw new Error("missing composer bindings");
    for (const element of text.textElements) expect(input.text.slice(element.startUtf16, element.endUtf16)).not.toBe("");
  });

  it("honors a disabled task skill instead of silently falling back to another prompt", async () => {
    localStorage.setItem("mock-task-skill:edit-structured-resources", "disabled");
    try { await expect(mockInternalTaskInput(context, target, payload, localStorage)).rejects.toThrow("需要启用"); }
    finally { localStorage.removeItem("mock-task-skill:edit-structured-resources"); }
  });
});
