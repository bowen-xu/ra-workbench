/** @vitest-environment happy-dom */
import { act, type ReactElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import fixtures from "../../../../../../tests/fixtures/math-delimiters.json";
import { AgentMarkdown } from "./AgentMarkdown";

describe("Agent formula delimiters", async () => {
  for (const fixture of fixtures) {
    it(`renders formulas without interpreting code: ${fixture.source}`, async () => {
      const host = document.createElement("div");
      host.innerHTML = await renderMarkup(<AgentMarkdown value={fixture.source} />);
      expect(host.querySelectorAll(".katex-error")).toHaveLength(0);
      const formulas = [...host.querySelectorAll(".katex")];
      const source = (node: Element) => node.querySelector("annotation")?.textContent;
      expect(formulas.filter(node => !node.closest(".katex-display")).map(source)).toEqual(fixture.legacy ? [] : fixture.inline);
      expect(formulas.filter(node => node.closest(".katex-display")).map(source)).toEqual(fixture.legacy ? [] : fixture.display);
      if (fixture.legacy) {
        expect(host.textContent).toContain("$x$");
        expect(host.textContent).toContain("$$\ny^2\n$$");
      }
    });
  }

  it("retains literal code and ordinary link destinations", async () => {
    const host = document.createElement("div");
    host.innerHTML = await renderMarkup(<AgentMarkdown value={'Show `\\(x\\)` and [link](https://example.com/\\(id\\)).'} />);
    expect(host.querySelector("code")?.textContent).toBe(String.raw`\(x\)`);
    expect(host.querySelector(".katex")).toBeNull();
    expect(host.querySelector("a")?.getAttribute("href")).toBe("https://example.com/(id)");
  });

  it("wraps complete formulas once, retaining accessible math and KaTeX script proportions", async () => {
    const host = document.createElement("div");
    host.innerHTML = await renderMarkup(<AgentMarkdown value={'行内 \\(E=mc^2\\)\n\n\\[\n\\frac{a}{b}\n\\]'} />);
    const viewports = [...host.querySelectorAll(".agent-math-viewport")];
    expect(viewports).toHaveLength(2);
    expect(viewports.map(node => node.getAttribute("data-math-display"))).toEqual(["false", "true"]);
    expect(viewports.every(node => node.querySelectorAll(".katex").length === 1)).toBe(true);
    expect(host.querySelectorAll("math")).toHaveLength(2);
    expect(host.querySelector(".katex-sizing.reset-size6.size3")).not.toBeNull();
    expect(host.querySelector(".agent-math-viewport .agent-math-viewport")).toBeNull();
  });
});

it.each([false,true])("keeps inline math distinct from display math within list text (streaming=%s)",async streaming=>{
 const value=String.raw`1. **组合优化**：例如在${"  "}
   \[
   (C_1 \wedge C_2 \wedge \cdots \wedge C_n)\Rightarrow R
   \]
   中，要学习哪些 \(C_i\) 应被纳入条件。
2. **连续优化**：学习参数 \(f,c\)。`;
 const host=document.createElement("div");
 host.innerHTML=await renderMarkup(<AgentMarkdown value={value} streaming={streaming}/>);
 expect(host.querySelectorAll("ol > li")).toHaveLength(2);
 const math=[...host.querySelectorAll(".agent-math-viewport")];
 expect(math.map(e=>e.getAttribute("data-math-display"))).toEqual(["true","false","false"]);
 expect(math.map(e=>e.querySelector("annotation")?.textContent)).toEqual([String.raw`(C_1 \wedge C_2 \wedge \cdots \wedge C_n)\Rightarrow R`,"C_i","f,c"]);
 expect(math.every(e=>e.querySelectorAll(".katex").length===1)).toBe(true);
 expect(host.querySelector(".katex-error")).toBeNull();
});

it.each([false, true])("renders a standalone equality inside display math without creating a Markdown heading (streaming=%s)", async streaming => {
 const formula = String.raw`\operatorname{score}(r)
=
\operatorname{new\_coverage}(r)
-\lambda\,\operatorname{complexity}(r)
-\mu\,\operatorname{conflict}(r)
-\nu\,\operatorname{redundancy}(r),`;
 const host = document.createElement("div");
 host.innerHTML = await renderMarkup(<AgentMarkdown value={`before\n\n\\[\n${formula}\n\\]\n\nafter`} streaming={streaming}/>);
 expect(host.querySelectorAll(".katex-display")).toHaveLength(1);
 expect(host.querySelector("annotation")?.textContent).toBe(formula);
 expect(host.querySelector("h1, h2, .katex-error")).toBeNull();
 expect(host.textContent).toContain("before"); expect(host.textContent).toContain("after");
});

async function renderMarkup(element: ReactElement): Promise<string> {
 const host=document.createElement("div"), root=createRoot(host);
 await act(async()=>root.render(element));
 const result=host.innerHTML;
 await act(async()=>root.unmount());
 return result;
}
