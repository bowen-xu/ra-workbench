import { expect, test, type Page } from "@playwright/test";

async function openConversation(page: Page) {
  await page.setViewportSize({width: 1280, height: 720});
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureAgentPreview({delayMs: 0, nextFailure: null}));
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  for (const [index, text] of ["合成输入稳定性样例一", "合成输入稳定性样例二"].entries()) {
    await input.fill(text);
    await page.getByRole("button", {name: "发送消息", exact: true}).click();
    await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
    const replies = page.locator(".writing-agent-message.assistant .writing-agent-markdown");
    await expect(replies).toHaveCount(index+1);
    await expect(replies.last()).toHaveAttribute("aria-busy", "false");
    await expect(replies.last()).not.toHaveAttribute("data-stream-revealing", "true");
  }
}

  test(`App: typing within existing lines preserves message position`, async ({page}) => {
    await openConversation(page);
    const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
    await input.fill("第一行合成文字\n第二行合成文字\n第三行合成文字");
    await page.evaluate(() => document.fonts.ready);
    const list = page.locator(".writing-agent-messages");
    await expect.poll(() => list.evaluate(node => node.scrollHeight-node.clientHeight)).toBeGreaterThan(10);
    const baseline = await list.evaluate(node => {
      node.scrollTop = node.scrollHeight-node.clientHeight;
      node.dispatchEvent(new Event("scroll"));
      return node.scrollTop;
    });
    await input.press("End");
    const motion = await list.evaluateHandle(node => {
      const samples: number[] = [];
      let frame = 0;
      const sample = () => { samples.push(node.scrollTop); frame=requestAnimationFrame(sample); };
      sample();
      return {samples, stop:()=>cancelAnimationFrame(frame)};
    });
    await input.pressSequentially("abcd", {delay: 60});
    const positions = await motion.evaluate(value => {value.stop();return value.samples;});
    await motion.dispose();
    expect(positions.length).toBeGreaterThan(1);
    expect(positions.every(position=>Math.abs(position-baseline)<.5)).toBe(true);
    await expect.poll(() => list.evaluate(node => node.scrollTop)).toBeCloseTo(baseline, 0);
    const height = await input.evaluate(node => node.getBoundingClientRect().height);
    const before = await list.evaluate(node => { node.scrollTop=Math.max(1,node.scrollTop/2); node.dispatchEvent(new Event("scroll")); return node.scrollTop; });
    await input.pressSequentially("efgh", {delay:60});
    expect(await input.evaluate(node=>node.getBoundingClientRect().height)).toBe(height);
    expect(await list.evaluate(node=>node.scrollTop)).toBeCloseTo(before, 0);
    await page.locator(".writing-agent-panel").screenshot({path:".tmp/composer-stable-app.png"});
  });

  test(`App: input grows, shrinks and remains inside the shared cap`, async ({page}) => {
    await openConversation(page);
    const input=page.getByRole("textbox",{name:"给研究助手发送消息",exact:true});
    await input.fill("一行");
    const minimum=await input.evaluate(node=>node.getBoundingClientRect().height);
    await input.fill("自动换行内容".repeat(20));
    expect(await input.evaluate(node=>node.getBoundingClientRect().height)).toBeGreaterThan(minimum);
    await input.fill("一\n二\n三");
    expect(await input.evaluate(node=>node.getBoundingClientRect().height)).toBeGreaterThan(minimum);
    await input.fill(Array(30).fill("长输入内容").join("\n"));
    const capped=await page.locator(".writing-agent-composer").evaluate(form=>({form:form.getBoundingClientRect().height,panel:form.closest(".writing-agent-panel")!.getBoundingClientRect().height}));
    expect(capped.form).toBeLessThanOrEqual(capped.panel*.35+1);
    await expect(input).toHaveCSS("overflow-y","auto");
    await input.fill("一行");
    await expect.poll(()=>input.evaluate(node=>node.getBoundingClientRect().height)).toBe(minimum);
    await expect(input).toHaveCSS("overflow-y","hidden");
    expect(await page.locator(".writing-agent-composer textarea").count()).toBe(1);
  });
