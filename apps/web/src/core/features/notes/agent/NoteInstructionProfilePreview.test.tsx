/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import type { AgentSubmissionInput, AgentInstructionFragment } from "@research-assistant/core/agent";
import { NoteInstructionProfilePreview } from "./NoteInstructionProfilePreview";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const base: AgentSubmissionInput = { text: "Question", model: {providerId:"test", modelId:"fixture", revision:null}, reasoningEffort:null, context:[] };
const fragment: AgentInstructionFragment = { environmentId:"note:fixture", scopeId:"note:fixture", sourceId:"agent/config.toml#profiles.note", revision:"a".repeat(64), originalBytes:12, text:"<b>rule</b>", disposition:"truncated" };
const config = { environment_id:fragment.environmentId, layers:[{origin:{tier:"user", source_id:fragment.sourceId, revision:fragment.revision}, instructions:{max_bytes:10, fallback_filenames:null}}] };

it("discloses only frozen profile text, scope, revision and truncation without interpreting markup", async()=>{
  const container=document.createElement("div"), root=createRoot(container);
  try {
    await act(async()=>root.render(<NoteInstructionProfilePreview input={{...base,instructions:[fragment],instructionConfigs:[config]}}/>));
    expect(container.querySelector("summary")?.textContent).toBe("查看冻结的笔记指令");
    expect(container.querySelector("details")?.open).toBe(false);
    expect(container.querySelector("pre")?.textContent).toBe("<b>rule</b>");
    expect(container.querySelector("b")).toBeNull();
    expect(container.textContent).toContain("已裁剪 · 原始 12 字节 · 最终上限 10 字节");
    expect(container.textContent).toContain(fragment.revision);
    expect(container.textContent).toContain("note:fixture");
    await act(async()=>root.render(<NoteInstructionProfilePreview input={{...base,instructions:[{...fragment,text:"",disposition:"budgetExcluded"}],instructionConfigs:[config]}}/>));
    expect(container.textContent).toContain("预算排除");
    expect(container.textContent).toContain("没有正文发送给模型");
    expect(container.querySelector("pre")).toBeNull();
    await act(async()=>root.render(<NoteInstructionProfilePreview input={base}/>));
    expect(container.textContent).toBe("");
  } finally {await act(async()=>root.unmount());}
});
