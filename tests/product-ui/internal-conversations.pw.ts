import { expect, test } from "@playwright/test";

test("structured reading uses the internal catalog and remains an editable conversation", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.getByRole("button", {name: "对话列表", exact: true}).click();
  const drawer = page.getByRole("complementary", {name: "对话列表面板", exact: true});
  await drawer.getByRole("button", {name: "内部对话", exact: true}).click();
  await expect(drawer).toContainText("暂无内部对话");
  await page.evaluate(async () => {
    const service = "/ra-workbench/apps/web/src/core/services/aiAssistanceGateway.ts";
    const {getAiAssistanceGateway} = await import(/* @vite-ignore */ service);
    const ai = getAiAssistanceGateway();
    const portrait = (await ai.listCandidates()).find(candidate => candidate.candidateKind === "paper_portrait_v1");
    if (!portrait) throw new Error("missing synthetic portrait");
    await ai.startPaperAnalysis({clientRequestId: crypto.randomUUID(), paperId: portrait.evidenceBundle.items[0].paperId,
      attachmentIds: [...new Set(portrait.evidenceBundle.items.map(source => source.attachmentId))],
      overviewProfileSelectionRevision: 0, imageAssets: [],
      paperTemplateRef: {portraitCandidateId: portrait.id, portraitRevision: portrait.currentRevision}});
  });
  const row = drawer.locator("[data-conversation-id]").filter({hasText: "结构化阅读"});
  await expect(row).toHaveCount(1);
  await row.locator(":scope > button").click();
  await expect(page.locator(".writing-agent-panel:visible")).toContainText("paper_overview_unit_v1");
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  await expect(input).toBeEditable();
  await input.fill("继续修改，补充说明算法步骤");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.locator(".writing-agent-panel:visible")).toContainText("本轮已读取");
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
  // The selected internal chat cannot be injected into ordinary pages by metadata recovery.
  if (!await drawer.isVisible()) await page.getByRole("button", {name: "对话列表", exact: true}).click();
  await drawer.getByRole("button", {name: "返回普通对话", exact: true}).click();
  await expect(drawer.locator("[data-conversation-id]").filter({hasText: "结构化阅读"})).toHaveCount(0);
  await page.screenshot({path: ".tmp/internal-conversations.png"});
});
