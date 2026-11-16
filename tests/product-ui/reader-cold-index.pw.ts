import {expect,test} from "@playwright/test";

declare global {interface Window {__readerIndexCounts:{checks:number;writes:number;pages:number;characters:number}}}

test("the production preview reader extracts and writes a cold PDF index, then reuses it on hot return",async({page})=>{
  const errors:string[]=[];
  page.on("pageerror",error=>errors.push(error.message));
  await page.goto("/?preview=app");
  await expect(page.getByRole("row",{name:/Synthetic Research Preview/})).toBeVisible();
  await page.evaluate(async()=>{
    const {getLiteratureGateway}=await import("/plugins/ra-literature-library/src/public/literatureGateway.ts");
    const gateway=getLiteratureGateway(),check=gateway.isAttachmentIndexed.bind(gateway),write=gateway.indexAttachment.bind(gateway);
    window.__readerIndexCounts={checks:0,writes:0,pages:0,characters:0};
    gateway.isAttachmentIndexed=async id=>{window.__readerIndexCounts.checks++;return check(id);};
    gateway.indexAttachment=async(item,id,pages)=>{
      await write(item,id,pages);
      window.__readerIndexCounts.writes++;
      window.__readerIndexCounts.pages=pages.length;
      window.__readerIndexCounts.characters=pages.reduce((sum,page)=>sum+page.text.length,0);
    };
  });
  await page.getByRole("row",{name:/Synthetic Research Preview/}).dblclick();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.",{exact:true})).toBeVisible();
  await expect.poll(()=>page.evaluate(()=>window.__readerIndexCounts.writes)).toBe(1);
  const cold=await page.evaluate(()=>window.__readerIndexCounts);
  expect(cold.checks).toBe(1);expect(cold.pages).toBeGreaterThan(0);expect(cold.characters).toBeGreaterThan(0);
  await page.getByRole("button",{name:"笔记",exact:true}).click();
  await page.getByRole("button",{name:"文献",exact:true}).click();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.",{exact:true})).toBeVisible();
  expect(await page.evaluate(()=>window.__readerIndexCounts)).toEqual(cold);
  expect(errors).toEqual([]);
  await page.screenshot({path:".tmp/reader-cold-index.png"});
});
