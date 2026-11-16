import { expect, test, type Page } from "@playwright/test";

const firstTitle = "Synthetic Research Preview: Evidence and Autonomy";
const secondTitle = "Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic";
const reader = (page: Page, title: string) => page.getByLabel(`${title} PDF 阅读器`, { exact: true });
const tabs = (page: Page) => page.getByRole("tablist", { name: "打开的标签页", exact: true });
const home = (page: Page) => tabs(page).getByRole("tab", { name: "返回文献列表", exact: true });
const files = (page: Page) => tabs(page).locator('.workspace-tab:not([data-home="true"]) > [role="tab"]');
const toolbar = (page: Page) => page.getByRole("banner", { name: "窗口工具栏", exact: true });

async function open(page: Page, title: string) {
  await page.getByRole("row", { name: new RegExp(title) }).dblclick();
  await expect(reader(page, title)).toBeVisible();
  await expect(reader(page, title).locator(".pdf-view .textLayer span").first()).toBeVisible();
}

test("workspace switches preserve readers, while Home and PDF visits enter back/forward history", async ({ page }) => {
  await page.goto("/?preview=app");
  await expect(home(page).locator("..")).toHaveCSS("height", "26px");
  await open(page, firstTitle);
  const firstInstance = await reader(page, firstTitle).elementHandle();
  await home(page).click();
  await open(page, secondTitle);
  const secondInstance = await reader(page, secondTitle).elementHandle();
  await tabs(page).getByRole("tab", { name: firstTitle, exact: true }).click();
  await toolbar(page).getByRole("button", { name: "后退", exact: true }).click();
  await expect(reader(page, secondTitle)).toBeVisible();
  await toolbar(page).getByRole("button", { name: "后退", exact: true }).click();
  await expect(page.locator(".library-surface")).toBeVisible();
  await toolbar(page).getByRole("button", { name: "前进", exact: true }).click();
  await expect(reader(page, secondTitle)).toBeVisible();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await expect(tabs(page)).toBeVisible();
  await expect(tabs(page).getByRole("tab", {name: secondTitle, exact: true})).toHaveAttribute("aria-selected", "false");
  await expect(reader(page, secondTitle)).toBeHidden();
  expect(await secondInstance!.evaluate(element => element.isConnected)).toBe(true);
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(reader(page, secondTitle)).toBeVisible();
  expect(await firstInstance!.evaluate(element => element.isConnected)).toBe(true);
  await expect(toolbar(page).getByRole("button", { name: "前进", exact: true })).toBeDisabled();
  await home(page).click();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(page.locator(".library-surface")).toBeVisible();
  await expect(files(page)).toHaveCount(2);
  await page.screenshot({ path: ".tmp/library-home-tabs.png" });
});

test("a fresh page restores tab order, active PDF, reading position and zoom; closed tabs stay closed", async ({ page }) => {
  await page.goto("/?preview=app");
  await open(page, secondTitle);
  const pdf = reader(page, secondTitle);
  const pageInput = pdf.getByRole("textbox", { name: "页码", exact: true });
  await pageInput.fill("2"); await pageInput.press("Enter");
  await expect(pageInput).toHaveValue("441"); // The sample PDF labels physical page 2 as 441.
  await pdf.getByRole("button", { name: "放大", exact: true }).click();
  const zoom = await pdf.locator(".reader-zoom-label").textContent();
  await home(page).click(); // Hiding flushes reading-state persistence.
  await open(page, firstTitle);
  await tabs(page).getByRole("tab", { name: secondTitle, exact: true }).click();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.reload();
  await expect(files(page)).toHaveText([new RegExp(secondTitle), new RegExp(firstTitle)]);
  await expect(pdf).toBeVisible();
  await expect(pdf.locator(".pdf-view .textLayer span").first()).toBeVisible();
  await expect(pageInput).toHaveValue("441");
  await expect(pdf.locator(".reader-zoom-label")).toHaveText(zoom!);
  await tabs(page).getByRole("button", { name: `关闭 ${firstTitle}`, exact: true }).click();
  await home(page).click();
  await page.reload();
  await expect(page.locator(".library-surface")).toBeVisible();
  await expect(files(page)).toHaveText([new RegExp(secondTitle)]);
  await expect(toolbar(page).getByRole("button", { name: "后退", exact: true })).toBeDisabled();
  await expect(toolbar(page).getByRole("button", { name: "前进", exact: true })).toBeDisabled();
});

test("an unavailable restored tab does not prevent other tabs from reopening", async ({ page }) => {
  await page.goto("/?preview=app");
  await open(page, firstTitle);
  await page.evaluate(() => {
    const key = Object.keys(localStorage).find(key => key.startsWith("research-assistant.app-preview.v3:")
      && key.endsWith("research-assistant:library-session.v1"))!;
    const snapshot = JSON.parse(localStorage.getItem(key)!);
    snapshot.tabs.push({ itemId: "unavailable-paper", attachmentId: "unavailable-pdf", contentSha256: "a".repeat(64) });
    localStorage.setItem(key, JSON.stringify(snapshot));
  });
  await page.reload();
  await expect(reader(page, firstTitle)).toBeVisible();
  await expect(files(page)).toHaveCount(1);
  await expect(page.getByText("1 个文献标签未能恢复：文献或原附件不可用。", { exact: true })).toBeVisible();
});

test("the production browser route uses the same Home and retained library surface", async ({ page }) => {
  await page.goto("/");
  await expect(home(page)).toBeVisible();
  const surface = await page.locator(".library-workspace").elementHandle();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(tabs(page)).toBeVisible();
  await expect(home(page)).toHaveAttribute("aria-selected", "false");
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(home(page)).toBeVisible();
  expect(await surface!.evaluate(element => element.isConnected)).toBe(true);
});
