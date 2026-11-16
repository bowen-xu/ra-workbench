import { expect, test, type Page } from "@playwright/test";
import type { ImportJob } from "../../../ra-workbench-sdk/packages/core/src/literature";

interface DoiHarness {
  lookups: number;
  saves: number;
  rejections: number;
  failNext: boolean;
  releases: (() => void)[];
  jobs: () => Promise<readonly ImportJob[]>;
}
type HarnessWindow = Window & { __doiHarness: DoiHarness };

async function openLibrary(page: Page, entry: string): Promise<void> {
  await page.goto(entry);
  await expect(page.locator(".literature-row").first()).toBeVisible();
  await page.evaluate(async () => {
    const modulePath = "/plugins/ra-literature-library/src/services/literatureGateway.ts";
    const { getLiteratureGateway } = await import(modulePath) as typeof import("../../plugins/ra-literature-library/src/services/literatureGateway");
    const gateway = getLiteratureGateway();
    const lookup = gateway.previewDoi.bind(gateway);
    const save = gateway.applyMetadataCandidate.bind(gateway);
    const reject = gateway.rejectMetadataCandidate.bind(gateway);
    const harness: DoiHarness = { lookups: 0, saves: 0, rejections: 0, failNext: false, releases: [],
      jobs: () => gateway.listImportJobs() };
    gateway.previewDoi = async (...args) => {
      harness.lookups++;
      await new Promise<void>(resolve => harness.releases.push(resolve));
      if (harness.failNext) { harness.failNext = false; throw new Error("HTTP 503 · request-doi-test"); }
      return lookup(...args);
    };
    gateway.applyMetadataCandidate = async (...args) => { harness.saves++; return save(...args); };
    gateway.rejectMetadataCandidate = async id => { harness.rejections++; await reject(id); };
    (window as HarnessWindow).__doiHarness = harness;
  });
}
async function submitTwice(page: Page): Promise<void> {
  await page.locator("#literature-import-popover").evaluate(form => {
    (form as HTMLFormElement).requestSubmit();
    (form as HTMLFormElement).requestSubmit();
  });
}
async function release(page: Page, index: number): Promise<void> {
  await page.evaluate(index => (window as HarnessWindow).__doiHarness.releases[index]?.(), index);
}

for (const entry of ["/?preview=app", "/?demo=app"]) test(`DOI lookup and confirmation reject repeated submissions: ${entry}`, async ({ page }) => {
  await openLibrary(page, entry);
  const originalCount = await page.locator(".literature-row").count();
  await page.getByRole("button", { name: "通过 DOI 添加", exact: true }).click();
  const form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/new");
  await submitTwice(page);
  await expect(form).toHaveAttribute("aria-busy", "true");
  await expect(form.getByRole("button", { name: "正在获取 DOI…" })).toBeDisabled();
  expect(await page.evaluate(() => (window as HarnessWindow).__doiHarness.lookups)).toBe(1);
  await page.screenshot({ path: `.tmp/verification/doi-loading-${entry.includes("demo") ? "demo" : "preview"}.png` });
  await release(page, 0);
  const confirmation = page.getByRole("dialog", { name: "手动添加文献", exact: true });
  await expect(confirmation).toBeVisible();
  await expect(page.getByText("正在导入 · 0/1", { exact: true })).toHaveCount(0);
  await expect(page.locator(".literature-row")).toHaveCount(originalCount);
  await page.screenshot({ path: `.tmp/verification/doi-confirmation-${entry.includes("demo") ? "demo" : "preview"}.png` });
  await submitTwice(page);
  await expect(confirmation).not.toBeVisible();
  await expect(page.locator(".literature-row")).toHaveCount(originalCount + 1);
  expect(await page.evaluate(() => (window as HarnessWindow).__doiHarness.saves)).toBe(1);
  const jobs = await page.evaluate(() => (window as HarnessWindow).__doiHarness.jobs());
  expect(jobs.filter(job => job.sourceKind === "doi")).toEqual([expect.objectContaining({ state: "completed", completedCount: 1 })]);
  await expect(page.getByText("模拟导入记录不存在", { exact: true })).toHaveCount(0);
});

test("cancelling and reopening a DOI form cannot install a late candidate", async ({ page }) => {
  await openLibrary(page, "/?preview=app");
  const trigger = page.getByRole("button", { name: "通过 DOI 添加", exact: true });
  await trigger.click();
  let form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/old"); await submitTwice(page);
  await form.getByRole("button", { name: "取消", exact: true }).click();
  await expect(form).not.toBeVisible();
  await trigger.click();
  form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/current"); await submitTwice(page);
  await release(page, 0);
  await expect.poll(() => page.evaluate(() => (window as HarnessWindow).__doiHarness.rejections)).toBe(1);
  await expect(form).toBeVisible();
  await expect(form.getByRole("textbox")).toHaveValue("10.1234/current");
  await release(page, 1);
  const confirmation = page.getByRole("dialog", { name: "手动添加文献", exact: true });
  await expect(confirmation).toBeVisible();
  await confirmation.getByRole("button", { name: "取消", exact: true }).click();
  await expect(confirmation).not.toBeVisible();
  const jobs = await page.evaluate(() => (window as HarnessWindow).__doiHarness.jobs());
  expect(jobs.filter(job => job.sourceKind === "doi").map(job => job.state)).toEqual(["cancelled", "cancelled"]);
  expect(await page.evaluate(() => (window as HarnessWindow).__doiHarness.saves)).toBe(0);
});

test("a lookup error releases the submit lock and preserves the DOI for retry", async ({ page }) => {
  await openLibrary(page, "/?preview=app");
  await page.getByRole("button", { name: "通过 DOI 添加", exact: true }).click();
  const form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/retry");
  await page.evaluate(() => { (window as HarnessWindow).__doiHarness.failNext = true; });
  await submitTwice(page); await release(page, 0);
  await expect(page.getByText("HTTP 503 · request-doi-test", { exact: true })).toBeVisible();
  await expect(form.getByRole("textbox")).toHaveValue("10.1234/retry");
  await expect(form.getByRole("button", { name: "检查并导入", exact: true })).toBeEnabled();
  await submitTwice(page); await release(page, 1);
  await expect(page.getByRole("dialog", { name: "手动添加文献", exact: true })).toBeVisible();
  expect(await page.evaluate(() => (window as HarnessWindow).__doiHarness.lookups)).toBe(2);
});

test("switching import entries rejects an existing DOI candidate before the next lookup", async ({ page }) => {
  await openLibrary(page, "/?preview=app");
  const trigger = page.getByRole("button", { name: "通过 DOI 添加", exact: true });
  await trigger.click();
  let form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/first");
  await submitTwice(page); await release(page, 0);
  await expect(page.getByRole("dialog", { name: "手动添加文献", exact: true })).toBeVisible();
  await trigger.click();
  form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await expect(form).toBeVisible();
  expect(await page.evaluate(() => (window as HarnessWindow).__doiHarness.rejections)).toBe(1);
  await form.getByRole("textbox").fill("10.1234/second");
  await submitTwice(page); await release(page, 1);
  await expect(page.getByRole("dialog", { name: "手动添加文献", exact: true })).toBeVisible();
  await submitTwice(page);
  await expect(page.getByRole("row", { name: /模拟 DOI 元数据/ })).toBeVisible();
  expect((await page.evaluate(() => (window as HarnessWindow).__doiHarness.jobs())).map(job => job.state)).toEqual(["cancelled", "completed"]);
});

test("an unresponsive lookup times out visibly and cannot reopen the editor later", async ({ page }) => {
  await openLibrary(page, "/?preview=app");
  await page.clock.install();
  await page.getByRole("button", { name: "通过 DOI 添加", exact: true }).click();
  const form = page.getByRole("dialog", { name: "通过 DOI 添加文献", exact: true });
  await form.getByRole("textbox").fill("10.1234/timeout");
  await submitTwice(page);
  await page.clock.fastForward(50_001);
  await expect(page.getByText(/DOI_LOOKUP_TIMEOUT/)).toBeVisible();
  await expect(form.getByRole("textbox")).toHaveValue("10.1234/timeout");
  await expect(form.getByRole("button", { name: "检查并导入", exact: true })).toBeEnabled();
  await release(page, 0);
  await expect.poll(() => page.evaluate(() => (window as HarnessWindow).__doiHarness.rejections)).toBe(1);
  await expect(form).toBeVisible();
  await expect(page.getByRole("dialog", { name: "手动添加文献", exact: true })).toHaveCount(0);
});
