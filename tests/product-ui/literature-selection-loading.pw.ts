import {expect, test} from "@playwright/test";

declare global {
  interface Window { __referenceProbe: {lists: string[]; details: number} }
}

test("literature selection does not read full parses behind collapsed references", async ({page}) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?preview=app");
  await expect(page.locator(".literature-row").first()).toBeVisible();
  const paper = await page.evaluate(async () => {
    const {getDoclingGateway} = await import("/ra-workbench/apps/web/src/core/services/doclingGateway.ts");
    const {getLiteratureGateway} = await import("/plugins/ra-literature-library/src/public/literatureGateway.ts");
    const item = (await getLiteratureGateway().list()).find(item => item.attachments.some(file => file.mimeType === "application/pdf"))!;
    const attachment = item.attachments.find(file => file.mimeType === "application/pdf")!;
    const gateway = getDoclingGateway();
    window.__referenceProbe = {lists: [], details: 0};
    const summary = {id: "selection-parse",itemId:item.id,attachmentId:attachment.id,contentSha256:attachment.contentSha256,
      serviceBaseUrl:"mock://selection",doclingStatus:"success",parseStatus:"success" as const,createdAt:"2026-10-24",finishedAt:"2026-10-24"};
    gateway.listParseRuns = async id => {window.__referenceProbe.lists.push(id);return id === item.id ? [summary] : [];};
    gateway.getParseRun = async () => {
      window.__referenceProbe.details++;
      return {...summary,jsonContent:{texts:[
        ...Array.from({length:10000},(_,index)=>({self_ref:`#/texts/${index}`,label:"text",text:"Synthetic document body ".repeat(20),prov:[{page_no:1}]})),
        {self_ref:"#/texts/10000",label:"reference",text:"Synthetic bibliography reference",prov:[{page_no:2}]},
      ]},rawResponse:{}};
    };
    return {id:item.id};
  });
  const selected = page.locator(`.literature-row[data-item-id="${paper.id}"]`);
  await selected.click();
  await expect(page.locator(".literature-detail-content[aria-busy=false]")).toBeVisible();
  await page.locator(`.literature-row:not([data-item-id="${paper.id}"])`).first().click();
  await expect(page.locator(".literature-detail-content[aria-busy=false]")).toBeVisible();
  await selected.click();
  await expect(page.locator(".literature-detail-content[aria-busy=false]")).toBeVisible();
  expect(await page.evaluate(() => window.__referenceProbe)).toEqual({lists:[],details:0});
  const summary = page.locator(".reference-verification-panel > summary");
  await summary.click();
  await expect(page.getByText("Synthetic bibliography reference",{exact:true})).toBeVisible();
  const opened = await page.evaluate(() => window.__referenceProbe);
  expect(opened.details).toBe(1);
  // React StrictMode may start two metadata reads; a retired effect must never
  // continue into the expensive full-document read.
  expect(opened.lists.length).toBeGreaterThan(0);
  expect(opened.lists.length).toBeLessThanOrEqual(2);
  expect(opened.lists.every(id => id === paper.id)).toBe(true);
  await summary.click(); await summary.press("Enter");
  await expect(page.getByText("Synthetic bibliography reference",{exact:true})).toBeVisible();
  expect(await page.evaluate(() => window.__referenceProbe)).toEqual(opened);
  await page.screenshot({path:".tmp/literature-selection-references.png"});
  expect(errors).toEqual([]);
});
