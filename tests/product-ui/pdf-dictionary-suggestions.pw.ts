import { expect, test } from "@playwright/test";

test("offline spelling candidates require a click and keep the PDF source unchanged", async ({ page }) => {
  const failures: string[] = [];
  const origin = new URL(test.info().project.use.baseURL as string).origin;
  page.on("pageerror", error => failures.push(error.message));
  await page.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== origin || /^\/(api|v1|rpc|tauri)(\/|$)/.test(url.pathname)) {
      failures.push(`Unexpected request: ${url.origin}${url.pathname}`); return route.abort();
    }
    return route.continue();
  });
  await page.goto("/?demo=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  const line = page.locator(".pdf-view .textLayer span").filter({ hasText: "Evidence, annotations, notes and manuscript editing share mock data." }).first();
  await expect(line).toBeVisible();
  const bounds = await line.evaluate(element => {
    const node = element.firstChild, offset = node?.textContent?.indexOf("Evidence") ?? -1;
    if (!node || offset < 0) throw new Error("Sample word is missing");
    const range = document.createRange(); range.setStart(node, offset); range.setEnd(node, offset + 7);
    const rect = range.getBoundingClientRect(); return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  });
  await page.mouse.move(bounds.x + .2, bounds.y + bounds.height / 2); await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width - .5, bounds.y + bounds.height / 2, { steps: 8 }); await page.mouse.up();
  const toolbar = page.getByLabel("选区操作", { exact: true });
  await toolbar.getByRole("button", { name: "离线查词", exact: true }).click();
  const dictionary = page.getByRole("region", { name: "离线词典", exact: true });
  await expect(dictionary).toContainText(/未找到“eviden(c)?”/);
  const candidates = dictionary.getByLabel("你可能想查", { exact: true });
  await expect(candidates).toBeVisible();
  await expect(dictionary.locator("article")).toHaveCount(0);
  await expect(dictionary.getByLabel("AI 语境释义", { exact: true })).toHaveCount(0);
  const annotations = await page.locator(".saved-highlight").count();
  await page.screenshot({ path: ".tmp/verification/dictionary-suggestions-browser.png" });
  await candidates.getByRole("button", { name: /^evidence拼写相近$/ }).click();
  await expect(dictionary.locator("article strong").first()).toHaveText("evidence");
  await expect(candidates).toBeVisible();
  expect(await candidates.getByRole("button").count()).toBeLessThanOrEqual(3);
  await expect(candidates.getByRole("button", { name: /^evidence拼写相近$/ })).toHaveCount(0);
  const candidateLayout = await candidates.evaluate(element => {
    const label = element.querySelector("span")?.getBoundingClientRect();
    const first = element.querySelector("button")?.getBoundingClientRect();
    if (!label || !first) throw new Error("Matched suggestions are missing");
    return { labelCenter: label.y + label.height / 2, candidateCenter: first.y + first.height / 2, labelRight: label.right, candidateLeft: first.left };
  });
  expect(Math.abs(candidateLayout.labelCenter - candidateLayout.candidateCenter)).toBeLessThanOrEqual(1);
  expect(candidateLayout.candidateLeft - candidateLayout.labelRight).toBeGreaterThanOrEqual(3);
  await page.screenshot({ path: ".tmp/verification/dictionary-hit-suggestions-browser.png" });
  await expect(line).toHaveText("Evidence, annotations, notes and manuscript editing share mock data.");
  expect(await page.locator(".saved-highlight").count()).toBe(annotations);
  expect(failures).toEqual([]);
});
