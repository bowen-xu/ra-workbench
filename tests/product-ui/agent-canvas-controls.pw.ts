import {expect, test, type Page} from "@playwright/test";
import {enqueueCanvasMessages} from "./agent-canvas-queue";
import {openLiveAgent, holdLiveAgentOutput} from "./live-agent";
import {openToolHistory, openToolJournal, openWritingJournal} from "./tool-journal";

async function startRunning(page: Page, writing = false) {
  const input = writing ? await openWritingJournal(page,false,true) : await openLiveAgent(page);
  await holdLiveAgentOutput(page);
  await input.fill("开始合成资料检查");
  await page.getByRole("button", {name:"发送消息", exact:true}).click();
  if (writing) {
    await expect(page.locator(".writing-agent-message.user").last()).toContainText("开始合成资料检查");
    const now = await page.evaluate(() => Date.now());
    await page.clock.pauseAt(new Date(now + 250));
    // This case needs an empty running composer after the real admission.
    await input.fill("");
  }
  await expect(page.getByRole("button", {name:"停止生成", exact:true})).toBeVisible();
}

test("queue menu has no hint and steering uses Chinese wording", async ({ page }) => {
  await page.setViewportSize({ width: 685, height: 808 });
  await page.clock.install();
  await startRunning(page,true);
  await enqueueCanvasMessages(page, ['修改 keywords 部分', '按百分比报告进度']);
  const row = page.locator('.agent-input-queue-body li').first();
  await row.hover();
  const menu = row.getByRole('button', { name: '第 1 条消息菜单', exact: true });
  await menu.hover();
  await page.waitForTimeout(650);
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  await expect(menu).not.toHaveAttribute('title');
  await menu.click();
  await expect(page.getByRole('menu', { name: '第 1 条消息菜单', exact: true })).toBeVisible();
  await page.keyboard.press('Escape');
  const steer = row.getByRole('button', { name: '将第 1 条消息插入当前轮次', exact: true });
  await expect(steer).toHaveText('引导');
  await steer.hover();
  await page.clock.runFor(350);
  await expect(page.getByRole('tooltip')).toHaveText('（非打断）提交引导');
  const caption = await page.locator('html').evaluate(element => getComputedStyle(element).getPropertyValue('--ui-font-caption').trim());
  await expect(page.getByRole('tooltip')).toHaveCSS('font-size', caption);
  await page.locator('.writing-agent-panel').screenshot({ path: '.tmp/agent-queue-hints.png' });
});

test("stopping retains a static elapsed status and divider before and after tool feedback", async ({ page }) => {
  await page.clock.install();
  await startRunning(page);
  const checkStopped = async () => {
    const status = page.locator('[data-agent-activity="aborted"]').last();
    await expect(status).toBeVisible();
    await expect(status).toContainText('已中止 ·');
    await expect(status).toHaveCSS('border-bottom-width', '1px');
    await expect(status).toHaveCSS('user-select', 'none');
    await expect(status.locator('svg')).toHaveCount(0);
    await expect(page.locator('[data-agent-activity="thinking"]')).toHaveCount(0);
    const process = status.locator('..').locator(':scope > .writing-agent-turn-process');
    await expect(process).toHaveCount(1);
    await expect(process).toHaveAttribute('data-flat', 'true');
    await expect(process).toHaveJSProperty('open', true);
    await expect(process).toHaveCSS('display', 'contents');
    await expect(process.locator(':scope > summary')).toBeHidden();
    const label = await status.textContent();
    await status.click();
    await page.clock.runFor(10_000);
    await expect(status).toHaveText(label!);
  };
  await page.clock.runFor(4_200);
  await page.getByRole('button', { name: '停止生成', exact: true }).click();
  await checkStopped();
  await openToolJournal(page,{outcome:"canceled",steps:[{name:"research_read_file",arguments:{path:"reading.md"},output:{name:"reading.md",text:"Synthetic evidence"}}]});
  await expect(page.locator('[data-agent-activity-record]')).not.toHaveCount(0);
  await checkStopped();
  await expect(page.locator('[data-agent-activity-record]').first()).toBeVisible();
  await page.screenshot({ path: '.tmp/agent-aborted-status.png', fullPage: true });
});

test("send and stop controls have no hints and preserve their actions", async ({ page }) => {
  await startRunning(page);
  const stop = page.getByRole('button', { name: '停止生成', exact: true });
  await expect(stop).toBeEnabled();
  expect(await stop.evaluate(element => element.parentElement?.className)).toBe('writing-agent-composer-primary');
  await expect(stop).not.toHaveAttribute('title');
  await stop.hover();
  await page.waitForTimeout(650);
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  await stop.click();
  await page.getByRole('textbox', { name: '给研究助手发送消息' }).fill('检查发送按钮');
  const send = page.getByRole('button', { name: '发送消息', exact: true });
  await expect(send).toBeEnabled();
  expect(await send.evaluate(element => element.parentElement?.className)).toBe('writing-agent-composer-primary');
  await expect(send).not.toHaveAttribute('title');
  await send.hover();
  await page.waitForTimeout(650);
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  await send.click();
  await expect(page.locator('.writing-agent-transcript')).toContainText('检查发送按钮');
  await expect(page.getByRole('button', { name: '停止生成', exact: true })).toBeVisible();
});

test("queue icon actions share hover styling and labels use captions at each width", async ({ page }) => {
  for (const width of [1172, 685, 632]) {
    await page.setViewportSize({ width, height: 808 });
    await startRunning(page);
    await enqueueCanvasMessages(page, ['修改 keywords 部分', '按百分比报告进度']);
    const row = page.locator('.agent-input-queue-body li').first();
    await expect(row).toHaveCSS('height', '25px');
    await expect(row.locator('.agent-input-queue-item-heading')).toHaveCSS('height', '25px');
    await row.hover();
    const remove = row.getByRole('button', { name: '移除第 1 条消息', exact: true });
    const menu = row.getByRole('button', { name: '第 1 条消息菜单', exact: true });
    for (const action of await row.locator('.agent-input-queue-inline-action, .agent-input-queue-item-more').all()) {
      await expect(action).toHaveCSS('height', '20px');
      await expect(action).toHaveCSS('min-height', '20px');
    }
    await expect(remove).toHaveCSS('border-width', '0px');
    await expect(menu).toHaveCSS('border-width', '0px');
    await expect(remove).toHaveCSS('width', '20px');
    await expect(menu).toHaveCSS('width', '20px');
    const style = (element: Element) => {
      const value = getComputedStyle(element);
      return { background: value.backgroundColor, color: value.color, border: value.border, radius: value.borderRadius, width: value.width, height: value.height };
    };
    await remove.hover();
    await expect(remove).toHaveCSS('border-width', '0px');
    const hovered = await remove.evaluate(style);
    await menu.hover();
    await expect(menu).toHaveCSS('border-width', '0px');
    expect(await menu.evaluate(style)).toEqual(hovered);
    await menu.click();
    await expect(menu).toHaveAttribute('aria-expanded', 'true');
    await expect(menu).toHaveCSS('border-width', '0px');
    await page.keyboard.press('Escape');
    const caption = await page.locator('html').evaluate(element => getComputedStyle(element).getPropertyValue('--ui-font-caption').trim());
    for (const label of [row.locator('p'), row.locator('.agent-input-queue-inline-action').first(), menu]) {
      await expect(label).toHaveCSS('font-size', caption);
    }
    for (const icon of await row.locator('.agent-input-queue-inline-action > svg, .agent-input-queue-item-more > svg').all()) {
      await expect(icon).toHaveCSS('width', caption);
      await expect(icon).toHaveCSS('height', caption);
    }
    for (const icon of [row.locator('.agent-input-queue-item-icon'), row.locator('.agent-input-queue-drag-handle svg')]) {
      await expect(icon).toHaveCSS('width', caption);
      await expect(icon).toHaveCSS('height', caption);
    }
    expect(await row.evaluate(element => {
        const bounds = element.getBoundingClientRect();
        return [...element.querySelectorAll('button')].filter(button => button.getClientRects().length > 0).every(button => {
          const rect = button.getBoundingClientRect();
          return rect.top >= bounds.top && rect.bottom <= bounds.bottom;
        });
    })).toBe(true);
    await row.hover();
    await page.locator('.writing-agent-composer-area').screenshot({ path: `.tmp/agent-task-icons-${width}.png` });
    await page.locator('.writing-agent-header').hover();
    await page.locator('.writing-agent-composer-area').screenshot({ path: `.tmp/agent-task-compact-${width}.png` });
    await page.screenshot({ path: `.tmp/agent-task-captions-${width}.png`, fullPage: true });
  }
});

test("full App uses the production busy slot and shared error notifications", async ({ page }) => {
  await page.goto('/?preview=app');
  await page.getByRole('button',{name:'打开对话',exact:true}).click();
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureAgentPreview({delayMs: 300, nextFailure: null}));
  const input=page.getByRole('textbox',{name:'给研究助手发送消息',exact:true});
  await input.fill('核对显示样式');
  await page.getByRole('button',{name:'发送消息',exact:true}).click();
  const checking=page.getByRole('status',{name:'输入框准备状态',exact:true});
  await expect(checking).toBeVisible();
  await expect(checking).toHaveText('正在核对请求……');
  await expect(page.locator('.writing-agent-messages [aria-label="输入框准备状态"]')).toHaveCount(0);
  await expect(page.locator('.agent-composer-input-state [aria-label="输入框准备状态"]')).toHaveCount(1);
  await expect(page.locator('.writing-agent-message.assistant')).toContainText('本轮已读取');
  await expect(page.getByRole('button',{name:'停止生成',exact:true})).toHaveCount(0);
  await page.getByRole("button",{name:"新建会话",exact:true}).click();
  await input.fill('失败后保留草稿');
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).holdAgentMetadata(true));
  await page.getByRole('button',{name:'发送消息',exact:true}).click();
  await expect.poll(() => page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).agentMetadataBlocked())).toBe(true);
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).releaseAgentMetadata());
  await expect(page.locator('[data-sonner-toast]')).toContainText('请求核对失败');
  await expect(input).toHaveText('失败后保留草稿');
  await expect(page.locator('.writing-agent-messages [role="alert"]')).toHaveCount(0);
});

test("the production composer keeps its preparing text inside the original input slot", async ({page}) => {
  const input = await openLiveAgent(page);
  await holdLiveAgentOutput(page);
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).holdAgentMetadata(false));
  await input.fill('检查发送状态');
  const inputHeight = await page.locator('.agent-composer-input-state').evaluate(element => element.getBoundingClientRect().height);
  await page.getByRole('button',{name:'发送消息',exact:true}).click();
  const checking = page.getByRole('status',{name:'输入框准备状态',exact:true});
  await expect(checking).toHaveText('正在核对请求……');
  const bodyFont = await page.locator(':root').evaluate(element => getComputedStyle(element).getPropertyValue('--ui-font-body').trim());
  await expect(checking).toHaveCSS('font-size',bodyFont);
  await expect(page.locator('[data-pending-submission="true"]')).toHaveText('检查发送状态');
  expect(await page.locator('.agent-composer-input-state').evaluate(element => element.getBoundingClientRect().height)).toBe(inputHeight);
  expect(await checking.evaluate(element => {
    const status = element.getBoundingClientRect(), composer = element.closest('form')!.getBoundingClientRect();
    return status.top >= composer.top && status.bottom <= composer.bottom && status.left >= composer.left && status.right <= composer.right;
  })).toBe(true);
  await expect(page.locator('.writing-agent-messages [aria-label="输入框准备状态"]')).toHaveCount(0);
  await expect(page.getByRole('button',{name:'停止发送',exact:true})).toBeEnabled();
  await page.locator('.writing-agent-panel').screenshot({path:'.tmp/submission-checking-production-style.png'});
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).releaseAgentMetadata());
  await expect(page.getByRole('button',{name:'停止生成',exact:true})).toBeVisible();
  await expect(page.locator('.writing-agent-message.user')).toHaveCount(1);
});

test("completed process headers prevent selection and animate expansion with reduced-motion support", async ({ page }) => {
  await openToolHistory(page);
  const header = page.locator('.writing-agent-turn-process > summary').first();
  await expect(header).toHaveCSS('user-select', 'none');
  const sample = await header.evaluate(async element => {
    (element as HTMLElement).click();
    await new Promise(requestAnimationFrame);
    const content = element.nextElementSibling as HTMLElement;
    const animation = content.getAnimations()[0]!;
    animation.pause();
    animation.currentTime = 80;
    const middle = { height: content.getBoundingClientRect().height, opacity: Number(getComputedStyle(content).opacity) };
    animation.finish();
    return { middle, fullHeight: content.getBoundingClientRect().height };
  });
  expect(sample.middle.height).toBeGreaterThan(0);
  expect(sample.middle.height).toBeLessThan(sample.fullHeight);
  expect(sample.middle.opacity).toBeGreaterThan(0);
  expect(sample.middle.opacity).toBeLessThan(1);
  await expect(page.locator('.writing-agent-activity-group > summary > .writing-agent-activity-status')).toHaveCount(0);
  await expect(page.locator('.writing-agent-activity-title').first()).toHaveCSS('user-select', 'text');
  await header.click();
  expect(await header.evaluate(async element => {
    (element as HTMLElement).click();
    await new Promise(requestAnimationFrame);
    const content = element.nextElementSibling as HTMLElement;
    const animation = content.getAnimations()[0]!;
    animation.pause();
    animation.currentTime = 40;
    (element as HTMLElement).click();
    await new Promise(requestAnimationFrame);
    return { open: (element.parentElement as HTMLDetailsElement).open, animations: content.getAnimations().length };
  })).toEqual({ open: false, animations: 0 });
  await page.emulateMedia({ reducedMotion: 'reduce' });
  await header.focus();
  await page.keyboard.press('Enter');
  await expect(header.locator('..')).toHaveAttribute('open', '');
  expect(await header.evaluate(element => element.nextElementSibling!.getAnimations().length)).toBe(0);
});

test("completed tools return to thinking before the next model response", async ({page}) => {
  const steps = [{name:"research_exec_command",arguments:{cmd:"printf evidence"},output:{exitCode:0,output:"evidence"}}];
  await openToolJournal(page,{steps,waitingForModel:true});
  await expect(page.locator('.writing-agent-activity-group.active')).toHaveCount(0);
  await expect(page.locator('[data-agent-activity="thinking"]')).toBeVisible();
  await expect(page.locator('.writing-agent-markdown')).toHaveCount(0);
  await expect(page.getByRole('region',{name:'写作提交状态',exact:true})).toHaveCount(0);
  await openToolJournal(page,{steps,response:'已检查稿件'});
  await expect(page.locator('.writing-agent-markdown')).toContainText('已检查稿件');
  await expect(page.locator('[data-agent-activity="thinking"]')).toHaveCount(0);
});

test("queue menu items stay left aligned and queue actions have no hints", async ({ page }) => {
  await startRunning(page);
  await enqueueCanvasMessages(page, ['修改 keywords 部分', '按百分比报告进度']);
  const row = page.locator('.agent-input-queue-body li').first();
  await row.hover();
  const handle = row.getByRole('button', { name: '拖动调整第 1 条消息顺序', exact: true });
  await expect(handle).not.toHaveAttribute('title');
  await handle.hover();
  await page.waitForTimeout(650);
  await expect(page.getByRole('tooltip')).toHaveCount(0);
  const remove = row.getByRole('button', {name:'移除第 1 条消息', exact:true});
  expect(await remove.evaluate(el => el.parentElement?.className)).toBe('agent-input-queue-item-actions');
  await row.getByRole('button', {name:'第 1 条消息菜单',exact:true}).click();
  const menu = page.getByRole('menu', {name:'第 1 条消息菜单',exact:true});
  await expect(menu).toBeVisible();
  for (const item of await menu.getByRole('menuitem').all()) {
    expect(await item.evaluate(el => ({align:getComputedStyle(el).justifyContent,padding:getComputedStyle(el).paddingLeft})))
      .toEqual({align:'flex-start',padding:'8px'});
  }
  await page.screenshot({path:'.tmp/agent-queue-menu-fixed.png',fullPage:true});
  await page.keyboard.press('Escape');
});

test("App writing responses can create and open a mock branch",async({page})=>{
 await openWritingJournal(page);
 const inherited = await page.locator(".writing-agent-message.assistant .writing-agent-markdown").last().innerText();
 const header=page.locator('.writing-agent-turn-process > .writing-agent-turn-header').first();
 await expect(header).toHaveCSS('padding-bottom','2px');
 await header.click();
 await expect(header).toHaveCSS('padding-bottom','2px');
 const branch=page.getByRole('button',{name:'从此轮创建分支（第 1 轮）',exact:true});
 await expect(branch).toBeAttached();
 await page.locator('.writing-agent-message.assistant').last().hover();
 await branch.click();
 const open=page.getByRole('button',{name:'打开分支（第 1 轮）',exact:true});
 await expect(open).toBeVisible();
 await open.click();
 await expect(page.getByLabel('分支来源',{exact:true})).toContainText('继续自');
 await expect(page.locator('.writing-agent-transcript')).toContainText(inherited);
});

test('pending tools shimmer only their outer label and thinking text uses the shared shimmer',async({page})=>{
 await startRunning(page);
 await expect(page.locator('[data-agent-activity="thinking"] .shimmer')).toHaveAttribute('data-shimmer-active','true');
 await openToolHistory(page,false,true);
 const group=page.locator('.writing-agent-activity-group.active').first();
 await expect(group).toBeVisible();
 const title=group.locator(':scope > summary > .writing-agent-activity-title');
 await expect(title).toHaveAttribute('data-shimmer-active','true');
 expect(await title.evaluate(el=>getComputedStyle(el).userSelect)).toBe('text');
 expect(await title.evaluate(el=>getComputedStyle(el).textOverflow)).toBe('ellipsis');
 await expect(group.locator('.writing-agent-activity-group-body [data-shimmer-active="true"]')).toHaveCount(0);
 await page.screenshot({path:'.tmp/agent-tool-shimmer.png',fullPage:true});
 await openToolHistory(page);
 await expect(page.locator('.writing-agent-activity-group.active')).toHaveCount(0);
});
