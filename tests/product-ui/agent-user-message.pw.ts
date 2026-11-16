import {expect, test} from "@playwright/test";
import {openLiveAgent} from "./live-agent";

test("long sent messages fold in the bubble and preserve tags through both copy paths", async ({page, context}) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.setViewportSize({width:1434,height:898});
  const composer = await openLiveAgent(page);
  await composer.fill("🧪 请结合以下引用检查稿件。\n");
  for (const file of ["main.tex", "sections/introduction.tex"]) {
    await page.getByRole("button", {name: "添加上下文和工具", exact: true}).click();
    await page.getByRole("menuitem", {name: "文件", exact: true}).click();
    await page.getByRole("menuitem").filter({hasText: file}).first().click();
  }
  await composer.press("End");
  await page.keyboard.insertText("\n" + Array.from({length: 20}, (_, index) => `第 ${index + 1} 项：检查研究问题、方法、证据和结论的对应关系，并保留引用来源。`).join("\n"));
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  const message=page.locator(".writing-agent-message.user").last();
  const bubble=message.locator(".writing-agent-message-bubble");
  const text=bubble.locator(".agent-user-message-text");
  const toggle=async(name:string)=>{
    await bubble.hover();
    const button=bubble.getByRole("button",{name,exact:true});
    await expect(button).toHaveCSS("opacity","1");
    await button.click();
  };
  await expect(bubble.getByRole("button",{name:"展开",exact:true})).toBeVisible();
  await expect(bubble.locator(".agent-user-message-ellipsis")).toHaveText("...");
  const foldedHeight=await text.evaluate(element=>element.getBoundingClientRect().height);
  expect(await text.textContent()).toContain("第 20 项");
  await toggle("展开");
  await expect(bubble.getByRole("button",{name:"收起",exact:true})).toBeVisible();
  expect(await text.evaluate(element=>element.getBoundingClientRect().height)).toBeGreaterThan(foldedHeight*2);
  await toggle("收起");
  await expect(text).toHaveAttribute("data-collapsed","true");
  await bubble.screenshot({path:".tmp/agent-long-message-folded.png"});
  const paste=async()=>{
    const data=await page.evaluate(async()=>{
      const items=await navigator.clipboard.read();
      return Promise.all(items.flatMap(item=>item.types.filter(type=>type==="text/plain"||type==="text/html").map(async type=>[type,await (await item.getType(type)).text()])));
    });
    expect(data.some(([type])=>type==="text/html")).toBe(true);
    await page.getByRole("textbox",{name:"给研究助手发送消息",exact:true}).evaluate((element,formats)=>{
      const clipboardData=new DataTransfer();
      for(const [type,value] of formats)clipboardData.setData(type!,value!);
      element.dispatchEvent(new ClipboardEvent("paste",{bubbles:true,cancelable:true,clipboardData}));
    },data);
    await expect(page.locator(".writing-agent-composer .agent-composer-reference")).toHaveCount(2);
    await expect(page.locator(".writing-agent-composer .cm-content")).toContainText("第 20 项");
  };
  await message.hover();
  await message.getByRole("button",{name:"复制消息",exact:true}).click();
  await expect(message.getByRole("button",{name:"已复制消息",exact:true})).toBeVisible();
  await paste();
  await page.locator(".writing-agent-composer .cm-content").focus();
  await page.keyboard.press("ControlOrMeta+A"); await page.keyboard.press("Backspace");
  await expect(page.locator(".writing-agent-composer .cm-placeholder")).toHaveText("有什么可以帮您");
  await expect(page.locator(".writing-agent-composer .agent-composer-reference")).toHaveCount(0);
  await toggle("展开");
  await text.locator("p").evaluate(element=>{
    const range=document.createRange();range.selectNodeContents(element);
    const selection=document.getSelection()!;selection.removeAllRanges();selection.addRange(range);
  });
  await page.keyboard.press("ControlOrMeta+C");
  await paste();
  await page.setViewportSize({width:685,height:808});
  await toggle("收起");
  await expect(text).toHaveAttribute("data-collapsed","true");
  expect(await text.evaluate(element=>element.getBoundingClientRect().height)).toBeLessThan(250);
});
