import { expect, test } from "@playwright/test";
import { equationSource as source } from "../ui/equation-source";

for (const url of ["/", "/?preview=app"]) {
  for (const kind of ["inline", "block"] as const) {
    test(`${kind} equation pairs brackets, wraps selection and completes LaTeX on ${url}`, async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.goto(url);
      await page.getByRole("button", { name: "笔记", exact: true }).click();
      await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
      await page.locator(".notes-content .bn-editor").click();
      await page.keyboard.type(kind === "inline" ? "$" : "$$"); await page.keyboard.press("Tab");
      if (kind === "block") await page.locator(".note-formula-disclosure").click();
      const input = page.getByRole("textbox", { name: kind === "inline" ? "行内公式 LaTeX 源码" : "公式 LaTeX 源码", exact: true });
      await expect(input).toBeFocused();
      for (const [open, close] of [["{", "}"], ["(", ")"], ["[", "]"]]) {
        await input.fill(""); await page.keyboard.type(open!);
        await expect.poll(() => source(input)).toBe(open! + close!);
        await input.press("Backspace"); await expect.poll(() => source(input)).toBe("");
        await page.keyboard.type(open! + "x" + close!);
        await expect.poll(() => source(input)).toBe(open! + "x" + close!);
        await input.fill("x+y"); await input.press("ControlOrMeta+a"); await page.keyboard.type(open!);
        await expect.poll(() => source(input)).toBe(open! + "x+y" + close!);
        await page.keyboard.type("z"); await expect.poll(() => source(input)).toBe(open! + "z" + close!);
      }
      await input.fill(""); await page.keyboard.type("\\alp");
      await expect(page.locator(".equation-source-completions").getByText("\\alpha", { exact: true })).toBeVisible();
      await input.press("Tab"); await expect.poll(() => source(input)).toBe("\\alpha");
      await expect(input).toBeFocused();

      await input.fill(""); await page.keyboard.type("\\fr");
      const option = page.locator(".equation-source-completions").getByText("\\frac", { exact: true });
      await expect(option).toBeVisible();
      const popup = await page.locator(".equation-source-completions").boundingBox();
      expect(popup!.x).toBeGreaterThanOrEqual(0);
      expect(popup!.y + popup!.height).toBeLessThanOrEqual(page.viewportSize()!.height);
      expect(await option.evaluate(element => {
        const bounds = element.getBoundingClientRect();
        return element.contains(document.elementFromPoint(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2));
      })).toBe(true);
      await page.screenshot({ path: test.info().outputPath(`${kind}-latex-completion.png`) });
      await option.click();
      await expect(input).toBeVisible(); await expect.poll(() => source(input)).toBe("\\frac{}{}");
      await expect(input).toBeFocused();
      await page.keyboard.type("a"); await input.press("Tab"); await page.keyboard.type("b");
      await expect.poll(() => source(input)).toBe("\\frac{a}{b}");
      await input.press("Shift+Tab"); await page.keyboard.type("c");
      await expect.poll(() => source(input)).toBe("\\frac{c}{b}");

      // Completion acceptance is one native undo step and does not save the equation.
      await input.fill(""); await page.keyboard.type("\\sq");
      await expect(page.locator(".equation-source-completions").getByText("\\sqrt", { exact: true })).toBeVisible();
      await input.press("Enter"); await expect.poll(() => source(input)).toBe("\\sqrt{}");
      await expect(input).toBeFocused(); await input.press("ControlOrMeta+z");
      await expect.poll(() => source(input)).toBe("\\sq");
      await expect(input).toBeVisible();

      await input.fill(""); await page.keyboard.type("\\\\");
      await expect.poll(() => source(input)).toBe("\\\\");
      await expect(page.locator(".equation-source-completions")).toHaveCount(0);

      await input.fill(""); await page.keyboard.type("\\alp");
      await expect(page.locator(".equation-source-completions")).toBeVisible();
      await input.dispatchEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true });
      await expect(input).toBeVisible(); await expect.poll(() => source(input)).toBe("\\alp");
      await input.press("Escape"); await expect(page.locator(".equation-source-completions")).toHaveCount(0);
      await expect(input).toBeFocused();
      await input.fill("x+y"); await input.press("Shift+Enter"); await input.press("Tab");
      await expect.poll(() => source(input)).toBe("x+y\n\t");
      await input.press("Enter"); await expect(input).toHaveCount(0);
      const trigger = page.locator(kind === "inline" ? ".inline-equation-preview" : ".note-formula-disclosure");
      await trigger.click(); await expect.poll(() => source(input)).toBe("x+y\n\t");
      await input.press("Escape"); await expect(input).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
}
