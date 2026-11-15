/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { WritingAgentActivityGroup } from "./WritingAgentActivity";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it("shimmers only the pending group label and retains every path for selection and copy", async () => {
 const container=document.createElement('div');document.body.append(container);const root=createRoot(container);
 try {
  await act(async()=>root.render(<WritingAgentActivityGroup items={[{type:'tool-call',name:'research_apply_patch',streaming:true,content:'{}'}]}/>));
  const label=container.querySelector('.tool-history-row .writing-agent-activity-title')!;
  expect(label.getAttribute('data-shimmer-active')).toBe('true');
  expect(container.querySelector('.writing-agent-activity-group > summary > .writing-agent-activity-status')).toBeNull();
  expect(container.querySelector('.tool-history-kind')?.getAttribute('data-shimmer-active')).toBeNull();
  expect(container.querySelector('.writing-agent-activity-group-body [data-shimmer-active]')).toBeNull();
  const paths=['sections/a-very-long-introduction.tex','sections/methods.tex','figures/results.tex'];
  await act(async()=>root.render(<WritingAgentActivityGroup items={[
   {type:'tool-call',name:'research_apply_patch',streaming:false,content:'{}'},
   {type:'tool-result',name:'research_apply_patch',streaming:false,content:JSON.stringify({changes:paths.map(path=>({path}))})},
  ]}/>));
  expect(label.getAttribute('data-shimmer-active')).toBe('false');
  expect(container.querySelector('.writing-agent-activity-group > summary > .writing-agent-activity-status')).toBeNull();
  expect(label.textContent).toContain("已运行 research_apply_patch");
  expect(container.querySelector("[data-agent-activity-group]")).toBeNull();
  const payload=container.querySelector('[aria-label="调用结果"]')!;
  for(const path of paths)expect(payload.textContent).toContain(path);
  const range=document.createRange();range.selectNodeContents(payload);expect(range.toString()).toContain(paths[2]);
 }finally{await act(async()=>root.unmount());container.remove();}
});
