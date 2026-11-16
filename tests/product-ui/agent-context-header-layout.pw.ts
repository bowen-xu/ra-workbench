import {expect,test,type Locator,type Page} from "@playwright/test";
import {PREVIEW_STORAGE_PREFIX} from "../../apps/web/src/core/runtime/appStorage";

async function openPaper(page: Page, title: string): Promise<void> {
  const row = page.getByRole("row").filter({has: page.getByText(title, {exact: true})});
  const label = row.getByText(title, {exact: true});
  await label.click();
  await expect(page.locator(".literature-detail").getByRole("heading", {name: title, exact: true})).toBeVisible();
  await label.dblclick();
  await expect(page.getByRole("tab", {name: title, exact: true})).toHaveAttribute("aria-selected", "true");
  // Tab selection precedes asynchronous PDF layout and scroll restoration;
  // Ariakit correctly cancels a pending hover hint on scroll.
  await expect(page.getByRole("region", {name: `${title} PDF 阅读器`, exact: true})
    .locator(".textLayer span").first()).toBeVisible();
}

async function expectUnclippedItalicInk(label: Locator): Promise<void> {
  const overflow=await label.evaluate(element=>{
    const style=getComputedStyle(element),context=document.createElement("canvas").getContext("2d");
    if(!context) throw new Error("Canvas text measurement unavailable");
    context.font=`${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
    context.textAlign="left";
    const ink=context.measureText(element.textContent ?? ""),range=document.createRange();
    range.selectNodeContents(element);
    return range.getBoundingClientRect().right+Math.max(0,ink.actualBoundingBoxRight-ink.width)-element.getBoundingClientRect().right;
  });
  // Italic ink can extend beyond its advance width; the clip must include it.
  expect(overflow).toBeLessThanOrEqual(.5);
}

test("text patch app preview preserves the research note's italic edge",async({page})=>{
  await page.goto("/?preview=app&demo=text-patch");
  const chip=page.getByRole("group",{name:"推荐持续上下文 笔记 · 研究笔记",exact:true});
  await expect(chip).toBeVisible();
  const remove=chip.getByRole("button",{name:"移除推荐上下文 笔记 · 研究笔记",exact:true});
  if(await remove.isVisible()) await remove.click();
  await expect(chip).toHaveAttribute("data-added","false");
  await page.mouse.move(20,20);
  const label=chip.locator(".agent-context-recommendation-label");
  await expect(label).toHaveCSS("font-style","italic");
  await expectUnclippedItalicInk(label);
  const bounds=(await chip.boundingBox())!;
  await page.screenshot({path:".tmp/agent-context-italic-note.png",clip:{x:bounds.x-16,y:bounds.y-12,width:bounds.width+72,height:46}});
});

for (const width of [1280,420]) for (const title of ["AI","Synthetic Research Preview: Evidence and Autonomy","Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic and Additional Context"]) {
  test(`context chip stays beside settings at ${width}px with ${title.length} title characters`,async({page})=>{
    await page.setViewportSize({width,height:808});
    await page.goto("/?preview=app");
    await expect(page.getByRole("row",{name:/Synthetic Research Preview/})).toBeVisible();
    // Only change an isolated preview fixture; production controls and layout stay in use.
    await page.evaluate(({key,title})=>{
      const items=JSON.parse(localStorage.getItem(key) ?? "[]") as {id:string;title:string}[];
      localStorage.setItem(key,JSON.stringify(items.map(item=>item.id === "10000000-0000-4000-8000-000000000001" ? {...item,title} : item)));
    },{key:PREVIEW_STORAGE_PREFIX+"research-assistant.literature.v1",title});
    await page.reload();
    const checkPopover=title === "AI" || title.length>80;
    if(checkPopover) {
      const other="Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic";
      await openPaper(page, other);
      await page.getByRole("tab",{name:"返回文献列表",exact:true}).click();
    }
    await openPaper(page, title);
    await page.getByRole("button",{name:"打开对话",exact:true}).click();
    const chip=page.getByRole("group",{name:`推荐持续上下文 ${title}`,exact:true});
    await expect(chip).toBeVisible();
    const metrics=()=>chip.evaluate(element=>{
      const header=element.closest(".writing-agent-header")!;
      const box=element.getBoundingClientRect();
      const wrapper=element.parentElement!.getBoundingClientRect();
      const settings=header.querySelector(".agent-context-settings")!.getBoundingClientRect();
      const bounds=header.getBoundingClientRect();
      const label=element.querySelector(".agent-context-recommendation-label")!;
      return {gap:settings.left-box.right,tail:wrapper.right-box.right,width:box.width,wrapperWidth:wrapper.width,left:box.left-bounds.left,right:settings.right-bounds.right,clipped:label.scrollWidth>label.clientWidth};
    });
    await expect.poll(async()=>Math.abs((await metrics()).gap-4)).toBeLessThan(1);
    const result=await metrics();
    expect(Math.abs(result.tail)).toBeLessThan(1);
    expect(result.width).toBeLessThanOrEqual(120);
    expect(result.wrapperWidth).toBeLessThanOrEqual(120);
    expect(result.left).toBeGreaterThanOrEqual(0);
    expect(result.right).toBeLessThan(0);
    if(title.length>40){expect(result.clipped).toBe(true);expect(result.width).toBe(120);}
    if(title === "AI")expect(result.width).toBeLessThan(100);
    if(checkPopover) {
      const disclosure=chip.getByRole("button",{name:`推荐上下文 ${title}`,exact:true});
      const indicator=chip.locator(".agent-context-recommendation-indicator");
      const label=chip.locator(".agent-context-recommendation-label");
      await page.mouse.move(20,20);
      await expect(indicator).toBeHidden();
      const resting=(await chip.boundingBox())!,text=(await label.boundingBox())!;
      // The text region reaches the ordinary trailing inset; the arrow must
      // overlap that region rather than enlarge either short or long chips.
      expect(resting.x+resting.width-text.x-text.width).toBeLessThanOrEqual(10);
      await disclosure.hover();
      await expect(indicator).toBeVisible();
      const arrow=(await indicator.boundingBox())!;
      expect(arrow.x).toBeLessThan(text.x+text.width);
      expect(Math.abs(arrow.y+arrow.height/2-resting.y-resting.height/2)).toBeLessThan(.5);
      const icon=(await chip.locator(".agent-context-recommendation-up").boundingBox())!;
      expect(Math.abs(icon.y+icon.height/2-resting.y-resting.height/2)).toBeLessThan(.5);
      expect((await chip.boundingBox())!.width).toBe(resting.width);
      const fade=()=>chip.evaluate(element=>({opacity:getComputedStyle(element,"::before").opacity,
        background:getComputedStyle(element,"::before").backgroundImage}));
      expect((await fade()).background).toContain("linear-gradient");
      await expect.poll(async()=>(await fade()).opacity).toBe("1");
      if(title === "AI") await page.locator(".writing-agent-panel:visible").screenshot({path:".tmp/agent-context-header-arrow-overlay.png"});
      await expect(page.locator(".control-hint:visible")).toContainText(title);
      await page.mouse.move(20,20);
      await expect(indicator).toBeHidden();
      await expect.poll(async()=>(await fade()).opacity).toBe("0");
      await disclosure.focus();
      await page.keyboard.press("Tab"); await page.keyboard.press("Shift+Tab");
      await expect(indicator).toBeVisible();
      await disclosure.click();
      const popup=page.getByRole("dialog",{name:"推荐上下文",exact:true});
      await expect(popup).toBeVisible();
      const expanded=(await chip.boundingBox())!,menu=(await popup.boundingBox())!;
      // A long candidate expands even a short primary name to the shared cap.
      expect(expanded.width).toBe(120);
      expect(Math.abs(expanded.x+expanded.width-resting.x-resting.width)).toBeLessThan(1);
      expect(Math.abs(menu.x-expanded.x)).toBeLessThan(1);
      expect(Math.abs(menu.width-expanded.width)).toBeLessThan(1);
      expect(Math.abs((await metrics()).gap-4)).toBeLessThan(1);
      if(title === "AI") expect(await label.evaluate(element=>{
        const range=document.createRange();range.selectNodeContents(element);
        return Math.abs(element.getBoundingClientRect().right-parseFloat(getComputedStyle(element).paddingRight)-range.getBoundingClientRect().right);
      })).toBeLessThan(1);
      await popup.getByRole("button",{name:/^添加持续上下文 /}).first().click();
      await page.keyboard.press("Escape");
      await expect(popup).toBeHidden();
      await page.getByRole("textbox",{name:"给研究助手发送消息",exact:true}).focus();
      await page.mouse.move(20,20);
      const count=chip.getByLabel("另有 1 项持续上下文",{exact:true});
      await expect(count).toBeVisible();
      const counted=(await chip.boundingBox())!,badge=(await count.boundingBox())!,name=(await label.boundingBox())!;
      // Count space remains reserved when hover temporarily replaces it with an arrow.
      expect(name.x+name.width).toBeLessThanOrEqual(badge.x);
      expect(counted.width).toBeLessThanOrEqual(120);
      expect(Math.abs(badge.y+badge.height/2-counted.y-counted.height/2)).toBeLessThan(.5);
      if(title === "AI") {
        expect(counted.width).toBeGreaterThan(resting.width);
        expect(counted.width).toBeLessThan(120);
        await page.locator(".writing-agent-panel:visible").screenshot({path:".tmp/agent-context-count-reserved.png"});
      }
      await disclosure.hover();
      await expect(count).toBeHidden();
      await expect(chip.locator(".agent-context-recommendation-up")).toBeVisible();
      expect((await chip.boundingBox())!.width).toBe(counted.width);
      await disclosure.click();
      await expect(popup).toBeVisible();
      await expect.poll(async()=>(await chip.boundingBox())!.width).toBe(120);
      const expandedName=(await label.boundingBox())!,expandedBadge=(await count.boundingBox())!;
      expect(expandedName.x+expandedName.width).toBeLessThanOrEqual(expandedBadge.x);
      if(title === "AI") expect(await label.evaluate(element=>{
        const range=document.createRange();range.selectNodeContents(element);
        return Math.abs(element.getBoundingClientRect().right-parseFloat(getComputedStyle(element).paddingRight)-range.getBoundingClientRect().right);
      })).toBeLessThan(1);
      if(title === "AI" && width === 1280) {
        const bounds=(await popup.boundingBox())!;
        await page.screenshot({path:".tmp/agent-context-candidates-expanded.png",clip:{x:expanded.x-10,y:bounds.y-10,width:expanded.width+80,height:bounds.height+expanded.height+20}});
      }
    }
  });
}

test("short candidates expand intrinsically without forcing the maximum width",async({page})=>{
  await page.goto("/?preview=app");
  await expect(page.getByRole("row",{name:/Synthetic Research Preview/})).toBeVisible();
  await page.evaluate(key=>{
    const items=JSON.parse(localStorage.getItem(key) ?? "[]") as {id:string;title:string}[];
    const titles: Record<string,string>={"10000000-0000-4000-8000-000000000001":"笔记 f","10000000-0000-4000-8000-000000000006":"研究笔记","10000000-0000-4000-8000-000000000008":"短"};
    localStorage.setItem(key,JSON.stringify(items.map(item=>titles[item.id] ? {...item,title:titles[item.id]} : item)));
  },PREVIEW_STORAGE_PREFIX+"research-assistant.literature.v1");
  await page.reload();
  for(const title of ["研究笔记","短"]) {
    await page.getByRole("row").filter({has:page.getByText(title,{exact:true})}).dblclick();
    await page.getByRole("tab",{name:"返回文献列表",exact:true}).click();
  }
  await page.getByRole("row").filter({has:page.getByText("笔记 f",{exact:true})}).dblclick();
  await page.getByRole("button",{name:"打开对话",exact:true}).click();
  const chip=page.getByRole("group",{name:"推荐持续上下文 笔记 f",exact:true});
  await chip.getByRole("button",{name:"移除推荐上下文 笔记 f",exact:true}).click();
  await expect(chip).toHaveAttribute("data-added","false");
  await page.mouse.move(20,20);
  await expectUnclippedItalicInk(chip.locator(".agent-context-recommendation-label"));
  const collapsed=(await chip.boundingBox())!;
  await chip.getByRole("button",{name:"推荐上下文 笔记 f",exact:true}).click();
  const popup=page.getByRole("dialog",{name:"推荐上下文",exact:true});
  await expect(popup).toBeVisible();
  const expanded=(await chip.boundingBox())!,menu=(await popup.boundingBox())!;
  expect(expanded.width).toBeGreaterThan(collapsed.width);
  expect(expanded.width).toBeLessThan(120);
  expect(menu.width).toBeCloseTo(expanded.width,0);
  const candidates=popup.locator(".agent-context-recommendation-label");
  await expect(candidates).toHaveCount(2);
  for(const label of [chip.locator(".agent-context-recommendation-label"),...await candidates.all()]) {
    await expectUnclippedItalicInk(label);
    expect(await label.evaluate(element=>element.scrollWidth <= element.clientWidth)).toBe(true);
    expect(await label.evaluate(element=>{
      const range=document.createRange();range.selectNodeContents(element);
      return Math.abs(element.getBoundingClientRect().right-parseFloat(getComputedStyle(element).paddingRight)-range.getBoundingClientRect().right);
    })).toBeLessThan(1);
  }
  const bounds=(await popup.boundingBox())!;
  await page.screenshot({path:".tmp/agent-context-all-right-aligned.png",clip:{x:expanded.x-10,y:bounds.y-10,width:expanded.width+80,height:bounds.height+expanded.height+20}});
  await page.keyboard.press("Escape");
  await expect(popup).toBeHidden();
  expect((await chip.boundingBox())!.width).toBe(collapsed.width);
});
