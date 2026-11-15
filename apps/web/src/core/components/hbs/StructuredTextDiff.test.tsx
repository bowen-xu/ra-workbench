// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect,it } from "vitest";
import { StructuredTextDiff } from "./StructuredTextDiff";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("preserves both complete Unicode texts while highlighting only changed ranges inside the card",async()=>{
  const host=document.createElement("div"),root=createRoot(host);
  const before="共同说明：旧算法🙂\n保留后文。",after="共同说明：新推理🧠\n保留后文。";
  try{
    await act(async()=>root.render(<StructuredTextDiff before={before} after={after}/>));
    expect(host.querySelector('[aria-label="修改前"]')?.textContent).toBe(before);
    expect(host.querySelector('[aria-label="修改后"]')?.textContent).toBe(after);
    expect(host.querySelector("del")?.textContent).not.toContain("共同说明");
    expect(host.querySelector("ins")?.textContent).not.toContain("保留后文");
  }finally{await act(async()=>root.unmount());}
});
it("does not invent a text diff for a metadata-only change",async()=>{
  const host=document.createElement("div"),root=createRoot(host);
  try{
    await act(async()=>root.render(<StructuredTextDiff before="正文不变" after="正文不变"/>));
    expect(host.querySelector("p")?.textContent).toBe("正文不变");
    expect(host.querySelector("del,ins")).toBeNull();
    expect(host.textContent).toContain("依据或状态已修改");
  }finally{await act(async()=>root.unmount());}
});
