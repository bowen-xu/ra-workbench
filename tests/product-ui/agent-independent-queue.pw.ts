import {expect,test,type Page} from "@playwright/test";
import {openLiveAgent, holdLiveAgentOutput} from "./live-agent";

async function open(page:Page) {
  const input = await openLiveAgent(page);
  await holdLiveAgentOutput(page, 3000);
  return input;
}

test("the production independent chat queues during streaming and dispatches each input once in order",async({page})=>{
  test.setTimeout(60_000);
  const input=await open(page);
  await input.fill("First question");
  await page.getByRole("button",{name:"发送消息",exact:true}).click();
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toBeVisible();
  await input.fill("Second question");
  await expect(page.getByRole("button",{name:"发送消息",exact:true})).toBeEnabled();
  await input.press("Enter");
  const queue=page.getByRole("region",{name:"队列消息",exact:true});
  await expect(queue).toContainText("Second question");
  await expect(input).toBeEmpty();
  await input.fill("Third question");
  await input.press("Enter");
  await expect(queue).toContainText("Third question");
  await expect(input).toBeEmpty();
  await input.fill("Unsent draft");
  await page.screenshot({path:".tmp/independent-conversation-queue.png"});
  await expect(page.locator(".writing-agent-message.assistant")).toHaveCount(3,{timeout:30_000});
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toHaveCount(0);
  await expect(input).toHaveText("Unsent draft");
  await expect(queue).toHaveCount(0);
  await expect(page.locator(".writing-agent-message.user .writing-agent-message-bubble")).toHaveText(["First question","Second question","Third question"]);
  const texts=await page.evaluate(async()=>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).readIndependentSubmissionTexts());
  expect(texts).toEqual(["First question","Second question","Third question"]);
});

test("stop and reload preserve queued input until the user resumes it",async({page})=>{
  test.setTimeout(60_000);
  const input=await open(page);
  await input.fill("First question");
  await page.getByRole("button",{name:"发送消息",exact:true}).click();
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toBeVisible();
  await input.fill("Keep this queued");await input.press("Enter");
  const queue=page.getByRole("region",{name:"队列消息",exact:true});
  await expect(queue).toContainText("Keep this queued");
  await page.getByRole("button",{name:"停止生成",exact:true}).click();
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toHaveCount(0);
  await expect(queue).toContainText("Keep this queued");
  await page.reload();
  const launcher=page.getByRole("button",{name:"打开对话",exact:true});
  await expect(launcher.or(input)).toBeVisible();
  if (await launcher.isVisible()) await launcher.click();
  await expect(queue).toContainText("Keep this queued");
  await expect(page.locator(".writing-agent-message.user")).toHaveCount(1);
  await queue.locator("li").first().hover();
  await page.getByRole("button",{name:"用第 1 条消息启动新轮次",exact:true}).click();
  await expect(page.locator(".writing-agent-message.user .writing-agent-message-bubble")).toHaveText(["First question","Keep this queued"],{timeout:20_000});
  await expect(queue).toHaveCount(0);
});

test("a running conversation survives management hiding and switching, and explicit stop cancels a new conversation",async({page})=>{
  test.setTimeout(60_000);
  const failures:string[]=[];page.on("pageerror",error=>failures.push(error.message));
  const firstText="Continue while management is hidden", secondText="Cancel this new conversation explicitly";
  await page.goto("/?preview=app");
  await page.getByRole("button",{name:"设置",exact:true}).click();
  await page.getByRole("button",{name:"高级",exact:true}).click();
  await page.getByRole("spinbutton",{name:"模拟延迟（毫秒）",exact:true}).fill("1000");
  await page.getByRole("button",{name:"保存模拟设置",exact:true}).click();
  await expect(page.locator("[data-sonner-toast]")).toContainText("模拟设置已保存");
  const input=page.getByRole("textbox",{name:"给研究助手发送消息",exact:true});
  const reveal=async()=>{const launcher=page.getByRole("button",{name:"打开对话",exact:true});if(await launcher.isVisible())await launcher.click();};
  // Inspect only the existing synthetic journal's envelope metadata. Text
  // remains in the production history/renderer and is not decoded here.
  const journal=()=>page.evaluate(async({firstText,secondText})=>{
    const {getRuntimeEnvironment}=await import("/ra-workbench/apps/web/src/core/public/runtimeEnvironment.ts");
    const storage=getRuntimeEnvironment().storage;
    const submissions=(JSON.parse(storage.getItem("independent-submissions")??"[]") as [string,{
      conversationId:string;requestId:string;turnId:string;input:{text:string};
    }][]).map(([,row])=>row).filter(row=>[firstText,secondText].includes(row.input.text))
      .map(({conversationId,requestId,turnId,input})=>({conversationId,requestId,turnId,text:input.text}));
    type Event={conversationId:string;requestId:string;payload:{type:string;turnId:string;outcome?:string}};
    const history=JSON.parse(storage.getItem("independent-history")??"null") as {
      events:{entries:[string,Event[]|{entries:Event[]}][]};
    }|null;
    const events=(history?.events.entries??[]).flatMap(([,value])=>Array.isArray(value)?value:value.entries)
      .filter(event=>submissions.some(row=>row.requestId===event.requestId))
      .map(({conversationId,requestId,payload})=>({conversationId,requestId,turnId:payload.turnId,type:payload.type,
        ...(payload.outcome?{outcome:payload.outcome}:{})}));
    return {selected:storage.getItem("research-assistant:independent-selected"),submissions,events};
  },{firstText,secondText});
  await page.getByRole("button",{name:"笔记",exact:true}).click();
  const manager=page.getByRole("complementary", {name: "笔记导航", exact: true});await expect(manager).toBeVisible();await reveal();
  await input.fill(firstText);await page.getByRole("button",{name:"发送消息",exact:true}).click();
  await expect.poll(async()=>(await journal()).events.filter(event=>event.type==="turnStarted").length,{timeout:15_000}).toBe(1);
  const started=await journal(), first=started.submissions[0]!;
  expect(started.submissions).toHaveLength(1);expect(started.selected).toBe(first.conversationId);
  expect(started.events.filter(event=>event.type==="turnFinished")).toEqual([]);
  await page.getByRole("button",{name:"隐藏对话",exact:true}).click();
  await page.getByRole("button",{name:"文献",exact:true}).click();await expect(manager).toBeHidden();
  const hidden=await journal();expect(hidden.selected).toBe(first.conversationId);
  expect(hidden.events.filter(event=>event.type==="turnFinished")).toEqual([]);
  await expect.poll(async()=>(await journal()).events.filter(event=>event.type==="textDelta").length)
    .toBeGreaterThan(started.events.filter(event=>event.type==="textDelta").length);
  await page.screenshot({path:".tmp/agent-manager-hidden-running.png"});
  await expect.poll(async()=>(await journal()).events.filter(event=>event.type==="turnFinished").map(event=>event.outcome))
    .toEqual(["completed"]);
  const completed=await journal();expect(completed.submissions).toEqual([first]);
  expect(completed.events.filter(event=>event.type==="turnStarted")).toMatchObject([
    {conversationId:first.conversationId,requestId:first.requestId,turnId:first.turnId},
  ]);
  expect(completed.events.filter(event=>event.type==="turnFinished")).toEqual([
    {conversationId:first.conversationId,requestId:first.requestId,turnId:first.turnId,type:"turnFinished",outcome:"completed"},
  ]);
  await page.getByRole("button",{name:"笔记",exact:true}).click();await expect(manager).toBeVisible();await reveal();
  await expect(page.locator(".writing-agent-message.assistant")).toContainText("本轮已读取");
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toHaveCount(0);
  // Cancellation is a separate user-created conversation, not evidence for a
  // second submission on the conversation that completed in the background.
  await page.getByRole("button",{name:"新建会话",exact:true}).click();
  await expect.poll(async()=>(await journal()).selected).not.toBe(first.conversationId);
  await expect(input).toBeEmpty();
  await input.fill(secondText);await page.getByRole("button",{name:"发送消息",exact:true}).click();
  await expect.poll(async()=>(await journal()).events.filter(event=>event.type==="turnStarted").length,{timeout:15_000}).toBe(2);
  const running=await journal(), second=running.submissions.find(row=>row.text===secondText)!;
  expect(second.conversationId).not.toBe(first.conversationId);expect(second.turnId).not.toBe(first.turnId);
  expect(second.requestId).not.toBe(first.requestId);
  expect(running.events.filter(event=>event.turnId===second.turnId&&event.type==="turnFinished")).toEqual([]);
  await page.getByRole("button",{name:"停止生成",exact:true}).click();
  await expect(page.getByRole("button",{name:"停止生成",exact:true})).toHaveCount(0);
  await expect.poll(async()=>(await journal()).events.filter(event=>event.turnId===second.turnId&&event.type==="turnFinished"))
    .toEqual([{conversationId:second.conversationId,requestId:second.requestId,turnId:second.turnId,type:"turnFinished",outcome:"canceled"}]);
  const stopped=await journal();expect(stopped.selected).toBe(second.conversationId);expect(stopped.submissions).toHaveLength(2);
  expect(stopped.events.filter(event=>event.type==="turnStarted")).toHaveLength(2);
  await test.info().attach("management-agent-lifecycle",{contentType:"application/json",body:JSON.stringify({
    hiddenWhileRunning:hidden.events.filter(event=>event.type==="turnFinished").length===0,
    backgroundTurn:completed.events.find(event=>event.type==="turnFinished"),
    canceledTurn:stopped.events.find(event=>event.turnId===second.turnId&&event.type==="turnFinished"),
    startedTurns:stopped.events.filter(event=>event.type==="turnStarted"),
  },null,2)});
  expect(failures).toEqual([]);
});
