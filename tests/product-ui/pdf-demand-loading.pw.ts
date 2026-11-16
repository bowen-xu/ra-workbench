import { expect,test } from "@playwright/test";

test("reading details reveal together while PDF and thumbnails remain independent", async ({page}) => {
  await page.setViewportSize({width:1434,height:898});
  await page.goto("/?preview=app");
  await expect(page.getByRole("row",{name:/Vision by Logic/})).toBeVisible();
  await page.evaluate(async () => {
    const path = "/ra-workbench/apps/web/src/core/services/doclingGateway.ts";
    const gateway = (await import(path)).getDoclingGateway();
    const original = gateway.listParseRuns.bind(gateway);
    let release: () => void = () => undefined;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const probe = {release, calls:0};
    Reflect.set(window,"readingDetailsProbe",probe);
    gateway.listParseRuns = async (id: string) => { probe.calls++; await gate; return original(id); };
  });
  const detail = page.locator(".literature-detail");
  await page.getByRole("row",{name:/Vision by Logic/}).click();
  await expect(detail.getByRole("status")).toHaveText("加载中…");
  await expect(detail.locator(".paper-reading-loading-spinner").first()).toHaveCSS("animation-name","reading-details-spin");
  await expect(detail.getByRole("heading",{name:/Vision by Logic/})).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"复制参考文献",exact:true})).toHaveCount(0);
  await page.screenshot({path:".tmp/verification/library-details-loading.png"});
  await page.getByRole("row",{name:/Synthetic Research Preview/}).click();
  await expect(detail.getByRole("status")).toHaveText("加载中…");
  await expect(detail.getByRole("heading")).toHaveCount(0);
  await page.getByRole("row",{name:/Vision by Logic/}).dblclick();
  const reader = page.getByLabel("Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic PDF 阅读器",{exact:true});
  await expect(reader.locator(".paper-reading-details-loading")).toHaveText("加载中…");
  await expect(reader.getByRole("region",{name:"文献画像",exact:true})).toHaveCount(0);
  await expect(reader.locator(".textLayer span").first()).toBeVisible();
  await expect(reader.getByRole("listbox",{name:"PDF 缩略图",exact:true}).locator("canvas:not(.pending)").first()).toBeVisible();
  await expect.poll(() => page.evaluate(() => Reflect.get(window,"readingDetailsProbe").calls)).toBeGreaterThan(0);
  await page.screenshot({path:".tmp/verification/reading-details-loading.png"});
  await page.evaluate(() => Reflect.get(window,"readingDetailsProbe").release());
  await expect(reader.locator(".paper-reading-details-loading")).toHaveCount(0);
  await expect(reader.getByRole("region",{name:"文献画像",exact:true})).toContainText("原创研究");
  await expect(reader.getByRole("heading",{name:/^结构化阅读记录/})).toBeVisible();
  await expect(reader.locator(".reference-verification-panel > summary")).toBeVisible();
  await page.screenshot({path:".tmp/verification/reading-details-ready.png"});
  await page.getByRole("tab",{name:"返回文献列表",exact:true}).click();
  await expect(detail.getByRole("heading",{name:/Vision by Logic/})).toBeVisible();
  await expect(detail.getByRole("button",{name:"复制引用",exact:true})).toBeVisible();
  await expect(detail.getByRole("button",{name:"归档此文献",exact:true})).toHaveCount(0);
  await expect(detail.getByRole("button",{name:"删除（移入回收站）",exact:true})).toHaveCount(0);
  await expect(detail.getByRole("status")).toHaveCount(0);
  await page.screenshot({path:".tmp/verification/library-details-ready.png"});
});

for(const entry of ["/?preview=app","/?demo=app"]){
  test(`reader panes load independently and thumbnails retain on-demand pixels across tabs: ${entry}`,async({page})=>{
    await page.setViewportSize({width:1434,height:898});
    let releasePdf:()=>void=()=>undefined;const pdfGate=new Promise<void>(resolve=>{releasePdf=resolve;});let pdfRequested=false;
    await page.route("**/preview/vision-by-logic.pdf",async route=>{pdfRequested=true;await pdfGate;await route.continue();});
    await page.goto(entry);
    await expect(page.getByRole("row",{name:/Vision by Logic/})).toBeVisible();
    await page.evaluate(async()=>{
      const path="/ra-workbench/apps/web/src/core/components/pdf-view/PdfViewAdapter.ts";
      const {PdfViewAdapter}=await import(path);
      const original=PdfViewAdapter.prototype.renderThumbnail;
      let release:()=>void=()=>undefined;const gate=new Promise<void>(resolve=>{release=resolve;});
      const probe={calls:[] as number[],finished:[] as number[],release};Reflect.set(window,"readerDemandProbe",probe);
      PdfViewAdapter.prototype.renderThumbnail=async function(pageNumber:number,canvas:HTMLCanvasElement,signal?:AbortSignal){
        canvas.dataset.renderCount=String(Number(canvas.dataset.renderCount ?? 0)+1);
        probe.calls.push(pageNumber);await gate;
        if(signal?.aborted)throw new DOMException("cancelled","AbortError");
        await original.call(this,pageNumber,canvas,signal);probe.finished.push(pageNumber);
      };
    });
    await page.getByRole("row",{name:/Vision by Logic/}).dblclick();
    const reader=page.getByLabel("Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic PDF 阅读器",{exact:true});
    await expect.poll(()=>pdfRequested).toBe(true);
    // Right-hand persisted content is usable even while PDF bytes cannot return.
    await expect(reader.getByRole("region",{name:"文献画像",exact:true})).toContainText("原创研究");
    await expect(reader.locator(".pdf-view canvas")).toHaveCount(0);
    releasePdf();
    await expect(reader.locator(".pdf-view .textLayer span").first()).toBeVisible();
    const thumbnails=reader.getByRole("listbox",{name:"PDF 缩略图",exact:true});
    await expect(thumbnails.getByRole("option")).toHaveCount(15);
    // Main text and the right pane do not wait for a single thumbnail to finish.
    await expect(thumbnails.locator("canvas:not(.pending)")).toHaveCount(0);
    const requested=await page.evaluate(()=>Reflect.get(window,"readerDemandProbe").calls as number[]);
    expect(requested.length).toBeGreaterThan(0);expect(requested.length).toBeLessThan(15);expect(requested).not.toContain(15);
    await page.evaluate(()=>Reflect.get(window,"readerDemandProbe").release());
    await expect.poll(()=>page.evaluate(()=>{const p=Reflect.get(window,"readerDemandProbe");return p.finished.length===p.calls.length;})).toBe(true);
    const first=thumbnails.getByRole("option",{name:"第 1 页",exact:true}).locator("canvas");
    await expect(first).not.toHaveClass("pending");const pixels=await first.evaluate(canvas=>(canvas as HTMLCanvasElement).toDataURL());
    const initialFirstRequests=await first.getAttribute("data-render-count");
    const readingOwner=await reader.locator(".paper-reading-details").elementHandle();
    await reader.getByRole("tab",{name:"高亮与笔记",exact:true}).click();
    const tabs=page.getByRole("banner",{name:"窗口工具栏",exact:true}).getByRole("tablist");
    await tabs.getByRole("tab",{name:"返回文献列表",exact:true}).click();
    await page.getByRole("row",{name:/Synthetic Research Preview/}).dblclick();
    await expect(page.locator('.reader-tab-surface:not([hidden]) .textLayer span').first()).toBeVisible();
    await tabs.getByRole("tab",{name:/Vision by Logic/}).click();
    await expect(reader).toBeVisible();await expect(first).not.toHaveClass("pending");
    expect(await readingOwner?.evaluate(node=>node.isConnected)).toBe(true);
    await expect(reader.getByRole("tab",{name:"高亮与笔记",exact:true})).toHaveAttribute("aria-selected","true");
    expect(await first.evaluate(canvas=>(canvas as HTMLCanvasElement).toDataURL())).toBe(pixels);
    await expect(first).toHaveAttribute("data-render-count",initialFirstRequests ?? "1");
    await expect(page.locator('.paper-reading-details[aria-busy="true"]')).toHaveCount(0);
    await page.evaluate(async()=>{
      const path="/ra-workbench/apps/web/src/core/runtime/appRuntime.ts", runtime=(await import(path)).getAppRuntime();
      const calls:Record<string,number>={};Reflect.set(window,"readerReopenCalls",calls);
      for(const name of ["getAiAssistanceGateway","getResearchKnowledgeGateway","getDoclingGateway"]){
        const gateway=runtime.gateways[name];
        for(const method of ["listCandidates","listReadingUnits","listParseRuns","getLatestPaperAnalysisWorkflow"]){
          if(typeof gateway[method]!=="function")continue;
          const original=gateway[method].bind(gateway);
          gateway[method]=async(...args:unknown[])=>{calls[method]=(calls[method]??0)+1;return original(...args);};
        }
      }
    });
    for(let i=0;i<2;i++){
      await tabs.getByRole("tab",{name:/Synthetic Research Preview/}).click();
      await tabs.getByRole("tab",{name:/Vision by Logic/}).click();
    }
    expect(await page.evaluate(()=>Reflect.get(window,"readerReopenCalls"))).toEqual({});
    await thumbnails.getByRole("option",{name:"第 15 页",exact:true}).scrollIntoViewIfNeeded();
    await expect(thumbnails.getByRole("option",{name:"第 15 页",exact:true}).locator("canvas")).not.toHaveClass("pending");
    await expect(reader).not.toContainText("startCleanup");
    if(entry.includes("preview"))await page.screenshot({path:".tmp/verification/pdf-demand-loading.png"});
  });
}
