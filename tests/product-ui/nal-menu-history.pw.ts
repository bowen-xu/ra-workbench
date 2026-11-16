import { expect, test } from "@playwright/test";
import { addBlock, editSlot, groupConnector } from "./nal-query-editor";

test("single-click connector menus match notes, commit immediately and restore focus without confirmation", async ({ page }) => {
  await page.goto("/nal-editor.html");
  await page.getByRole("button", { name: "从空白开始", exact: true }).click();
  const trigger = page.getByRole("button", { name: "条件 1 连接符", exact: true });
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  await trigger.click();
  const menu = page.getByRole("menu", { name: "条件 1 连接符", exact: true });
  await expect(menu).toHaveClass(/note-action-menu/);
  await expect(menu).toHaveCSS("border-radius", "8px");
  await expect(menu).toHaveCSS("background-color", "rgb(255, 255, 255)");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await menu.getByRole("menuitemradio", { name: "-->（继承）", exact: true }).click();
  await expect(menu).toHaveCount(0);
  await expect(expression).toHaveText("[选择词项] → [选择词项]");
  await expect(trigger).toBeFocused();
  await trigger.press("Enter");
  await expect(menu.getByRole("menuitemradio", { name: "-->（继承）", exact: true })).toHaveAttribute("aria-checked", "true");
  await menu.press("Escape");
  await trigger.press("ControlOrMeta+z");
  await expect(expression).toHaveText("[选择连接符]");
  await trigger.press("ControlOrMeta+Shift+z");
  await expect(expression).toHaveText("[选择词项] → [选择词项]");
  await trigger.click();
  const second = menu.getByRole("menuitemradio", { name: "==>（蕴含）", exact: true });
  await second.focus(); await page.keyboard.press("Enter");
  await expect(expression).toHaveText("[选择词项] ⇒ [选择词项]");
  await page.setViewportSize({ width: 420, height: 500 });
  await trigger.click();
  const bounds = await menu.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0); expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(420);
  expect(await second.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
  await page.screenshot({ path: ".tmp/verification/nal-connector-menu.png" });
  await menu.press("Escape");
});

test("undo/redo includes projection, compounds, block addition and branching edits while preserving cancelled popup drafts", async ({ page }) => {
  await page.goto("/nal-editor.html");
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  const undo = page.getByRole("button", { name: "撤销语句编辑", exact: true });
  const redo = page.getByRole("button", { name: "重做语句编辑", exact: true });
  await expect(undo).toBeDisabled();
  await editSlot(page, "条件 1 左槽 connector 1", "conjunction（∧）");
  await page.getByRole("button", { name: "移除输出变量 ?文章", exact: true }).click();
  await addBlock(page);
  await expect(page.getByRole("group", { name: "条件 2", exact: true })).toBeVisible();
  await undo.click();
  await expect(page.getByRole("group", { name: "条件 2", exact: true })).toHaveCount(0);
  await undo.click();
  await expect(page.getByRole("button", { name: "移除输出变量 ?文章", exact: true })).toBeVisible();
  await undo.click();
  await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await expect(undo).toBeDisabled();
  await redo.click(); await redo.click(); await redo.click();
  await expect(page.getByRole("group", { name: "条件 2", exact: true })).toBeVisible();
  await expect(expression).toContainText("∧");
  await expect(page.getByRole("button", { name: "添加输出变量 ?文章", exact: true })).toBeVisible();
  await undo.click();
  await editSlot(page, "条件 1 左槽 connector 1", "disjunction（∨）");
  await expect(redo).toBeDisabled();
  await undo.click();
  await expect(expression).toContainText("∧");

  const term = page.getByRole("button", { name: "条件 1 左槽 · 第 1 项", exact: true });
  await term.click();
  const editor = page.getByRole("menu", { name: "条件 1 左槽 · 第 1 项", exact: true });
  const variable = editor.getByRole("textbox", { name: "条件 1 左槽 · 第 1 项的变量名", exact: true });
  await variable.focus(); await variable.press("End"); await page.keyboard.type("2");
  await variable.press("ControlOrMeta+z");
  await expect(variable).toHaveValue("文章");
  await expect(expression).toContainText("?文章");
  await editor.press("Escape");
  await expect(redo).toBeEnabled();
  await page.getByRole("button", { name: "重置当前示例", exact: true }).click();
  await expect(undo).toBeDisabled(); await expect(redo).toBeDisabled();
});

test("nested statement menu selections and compound drag order participate in the same history", async ({ page }) => {
  await page.goto("/nal-editor.html");
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  const terms = expression.locator(".nal-compound-item");
  await terms.nth(0).dragTo(terms.nth(1));
  await expect(expression).toHaveText("(AI领域 × ?文章) → 所属领域");
  await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
  await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await page.getByRole("button", { name: "嵌套语句", exact: true }).click();
  await editSlot(page, "条件 1 左槽 连接符", "==>（蕴含）");
  await expect(expression).toContainText("?方法) ⇒ 基于方法");
  await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
  await expect(expression).toContainText("?方法) → 基于方法");
  await expect(page.getByRole("button", { name: "撤销语句编辑", exact: true })).toBeDisabled();
});

test("all internal choices commit from note menus and removing a focused block keeps the keyboard history chain", async ({ page }) => {
  await page.goto("/nal-editor.html");
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  const undo = page.getByRole("button", { name: "撤销语句编辑", exact: true });
  await page.getByRole("button", { name: "条件 1 左槽 connector 1", exact: true }).click();
  const menu = page.getByRole("menu", { name: "条件 1 左槽 connector 1", exact: true });
  await menu.getByRole("menuitem", { name: "增加槽位", exact: true }).click();
  await expect(page.getByRole("button", { name: "条件 1 左槽 · 第 3 项", exact: true })).toBeVisible();
  await undo.click();
  await expect(page.getByRole("button", { name: "条件 1 左槽 · 第 3 项", exact: true })).toHaveCount(0);
  await editSlot(page, "条件 1 左槽 connector 1", "disjunction（∨）");
  await expect(expression).toHaveText("(?文章 ∨ AI领域) → 所属领域");
  await undo.click();
  await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await expect(page.locator(".nal-query-editor select")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "应用词项", exact: true })).toHaveCount(0);

  await editSlot(page, "条件 1 连接符", "==>（蕴含）");
  await addBlock(page);
  await editSlot(page, "条件 2 连接符", "-->（继承）");
  await page.keyboard.press("ControlOrMeta+z");
  await page.keyboard.press("ControlOrMeta+z");
  await expect(page.getByRole("group", { name: "条件 2", exact: true })).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  for (let i = 0; i < 3; i++) await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(page.getByLabel("条件 2 Narsese 语句", { exact: true })).toHaveText("[选择词项] → [选择词项]");

  await page.getByRole("button", { name: "嵌套分组", exact: true }).click();
  await groupConnector(page, "分组 1 connector", "conjunction");
  await expect(page.getByRole("button", { name: "分组 1 connector", exact: true })).toHaveText("∧ conjunction");
  await undo.click();
  await expect(page.getByRole("button", { name: "分组 1 connector", exact: true })).toHaveText("∨ disjunction");
  await page.getByRole("button", { name: "条件 1.1 左槽 · 第 2 项", exact: true }).click();
  await page.getByRole("menuitem", { name: "条件 1.1 左槽 · 第 2 项 atomic", exact: true }).click();
  await page.screenshot({ path: ".tmp/verification/nal-all-choice-menu.png" });
});

test("production query history resets after save and nested menus stay usable inside the saved-query dialog", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
  await editSlot(page, "条件 1 连接符", "-->（继承）");
  await editSlot(page, "条件 1 左槽", "变量", "文章");
  await editSlot(page, "条件 1 右槽", "AI领域");
  await page.getByRole("switch", { name: "保存为查询集合", exact: true }).check();
  await page.getByLabel("查询集合名称", { exact: true }).fill("单击菜单与历史");
  await page.getByRole("button", { name: "查询并保存", exact: true }).click();
  await page.getByRole("button", { name: "查询操作：单击菜单与历史", exact: true }).click();
  await page.getByRole("menuitem", { name: "编辑查询：单击菜单与历史", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "编辑查询集合", exact: true });
  await expect(dialog.getByRole("button", { name: "撤销语句编辑", exact: true })).toBeDisabled();
  await dialog.getByRole("button", { name: "条件 1 右槽", exact: true }).click();
  await page.getByRole("menuitem", { name: "条件 1 右槽 atomic", exact: true }).click();
  await page.getByRole("menuitem", { name: "条件 1 右槽 concept", exact: true }).click();
  const item = page.getByRole("menu", { name: "条件 1 右槽 concept", exact: true }).getByRole("menuitemradio", { name: "深度学习领域", exact: true });
  expect(await item.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
  await item.click();
  const expression = dialog.getByLabel("条件 1 Narsese 语句", { exact: true });
  await expect(expression).toHaveText("?文章 → 深度学习领域");
  await page.keyboard.press("ControlOrMeta+z");
  await expect(expression).toHaveText("?文章 → AI领域");
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect(expression).toHaveText("?文章 → 深度学习领域");
});


test("atomic categories search concepts and papers in place without back rows, cascades or Tab expansion", async ({ page }) => {
  await page.goto("/nal-editor.html");
  const trigger = page.getByRole("button", { name: "条件 1 右槽", exact: true });
  const undo = page.getByRole("button", { name: "撤销语句编辑", exact: true });
  await trigger.click();
  const root = page.getByRole("menu", { name: "条件 1 右槽", exact: true });
  const atomic = root.getByRole("menuitem", { name: "条件 1 右槽 atomic", exact: true });
  await atomic.hover();
  await expect(page.getByRole("menu", { name: "条件 1 右槽 atomic", exact: true })).toHaveCount(0);
  await expect(root.locator(".lucide-chevron-right, kbd")).toHaveCount(0);
  await atomic.focus(); await page.keyboard.press("Tab");
  await expect(page.getByRole("menu", { name: "条件 1 右槽 atomic", exact: true })).toHaveCount(0);
  await page.keyboard.press("Escape"); await trigger.click();
  const id = await root.getAttribute("id");
  await root.getByRole("menuitem", { name: "条件 1 右槽 atomic", exact: true }).click();
  const categories = page.getByRole("menu", { name: "条件 1 右槽 atomic", exact: true });
  await expect(categories).toHaveAttribute("id", id!);
  await expect(categories.locator('[role^="menuitem"]')).toHaveText(["变量", "概念", "文献"]);
  await categories.getByRole("menuitem", { name: "条件 1 右槽 concept", exact: true }).click();
  const concepts = page.getByRole("menu", { name: "条件 1 右槽 concept", exact: true });
  const search = concepts.getByRole("searchbox", { name: "搜索概念", exact: true });
  await expect(concepts).toHaveAttribute("id", id!);
  await expect(search).toBeFocused();
  await expect(concepts.getByRole("menuitem", { name: "返回词项类型", exact: true })).toHaveCount(0);
  await search.fill("不存在");
  await expect(concepts.getByRole("status")).toHaveText("没有匹配的概念");
  await expect(undo).toBeDisabled();
  await search.fill("领域");
  await search.press("ArrowDown");
  await expect(search).toBeFocused();
  await page.screenshot({ path: ".tmp/verification/nal-concept-search.png" });
  await search.press("Enter");
  await expect(trigger).toHaveText("深度学习领域");
  await expect(trigger).toBeFocused();
  await undo.click();
  await trigger.click();
  await root.getByRole("menuitem", { name: "条件 1 右槽 atomic", exact: true }).click();
  await categories.getByRole("menuitem", { name: "条件 1 右槽 paper", exact: true }).click();
  const papers = page.getByRole("menu", { name: "条件 1 右槽 paper", exact: true });
  const paperSearch = papers.getByRole("searchbox", { name: "搜索文献", exact: true });
  await expect(paperSearch).toBeFocused();
  await paperSearch.fill("vision");
  await expect(papers.getByRole("menuitemradio")).toHaveText(["Vision by Logic"]);
  await page.setViewportSize({ width: 420, height: 760 });
  await expect(async () => {
    const rect = await papers.boundingBox();
    expect(rect!.x).toBeGreaterThanOrEqual(0); expect(rect!.x + rect!.width).toBeLessThanOrEqual(420);
  }).toPass();
  await page.screenshot({ path: ".tmp/verification/nal-paper-search.png" });
  await paperSearch.press("Escape");
  await expect(trigger).toBeFocused(); await expect(undo).toBeDisabled();
  await trigger.click();
  await root.getByRole("menuitem", { name: "条件 1 右槽 compound", exact: true }).click();
  const compound = page.getByRole("menu", { name: "条件 1 右槽 compound", exact: true });
  await expect(compound.getByRole("menuitemradio")).toHaveText(["product（×）", "conjunction（∧）", "disjunction（∨）"]);
  await expect(compound.getByRole("menuitem", { name: "返回词项类型", exact: true })).toHaveCount(0);
});

test("clear removes statement and compound contents; only undo redo restores the original terms", async ({ page }) => {
  await page.goto("/nal-editor.html");
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  const undo = page.getByRole("button", { name: "撤销语句编辑", exact: true });
  const redo = page.getByRole("button", { name: "重做语句编辑", exact: true });
  const clear = async (label: string) => {
    await page.getByRole("button", { name: label, exact: true }).click();
    await page.getByRole("menuitem", { name: "清空", exact: true }).click();
  };
  await clear("条件 1 连接符");
  await expect(expression).toHaveText("[选择连接符]");
  await expect(page.getByRole("button", { name: "移除输出变量 ?文章", exact: true })).toHaveCount(0);
  await undo.click(); await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await redo.click(); await expect(expression).toHaveText("[选择连接符]");
  await editSlot(page, "条件 1 连接符", "==>（蕴含）");
  await expect(expression).toHaveText("[选择词项] ⇒ [选择词项]");
  await undo.click(); await expect(expression).toHaveText("[选择连接符]");
  await undo.click(); await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await redo.click(); await redo.click();
  await expect(expression).toHaveText("[选择词项] ⇒ [选择词项]");

  await page.getByRole("button", { name: "重置当前示例", exact: true }).click();
  await clear("条件 1 左槽 connector 1");
  await expect(expression).toHaveText("[选择词项] → 所属领域");
  await expect(expression.locator(".nal-compound-item")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "条件 1 左槽 connector 1", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "条件 1 左槽", exact: true })).toBeFocused();
  await expect(page.getByRole("button", { name: "移除输出变量 ?文章", exact: true })).toHaveCount(0);
  await undo.click(); await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
  await redo.click();
  await editSlot(page, "条件 1 左槽", "disjunction（∨）");
  await expect(expression).toHaveText("([选择词项] ∨ [选择词项]) → 所属领域");
  await undo.click(); await undo.click();
  await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");

  await page.getByRole("button", { name: "嵌套语句", exact: true }).click();
  await clear("条件 1 左槽 连接符");
  await expect(expression).toHaveText("<[选择连接符]> ⇒ <(?文章 × AI领域) → 所属领域>");
  await editSlot(page, "条件 1 左槽 连接符", "-->（继承）");
  await expect(expression).toHaveText("<[选择词项] → [选择词项]> ⇒ <(?文章 × AI领域) → 所属领域>");
  await page.screenshot({ path: ".tmp/verification/nal-clear-nested-statement.png" });
  await undo.click(); await undo.click();
  await expect(expression).toContainText("(?文章 × ?方法) → 基于方法");
  await redo.click(); await redo.click();
  await expect(expression).toHaveText("<[选择词项] → [选择词项]> ⇒ <(?文章 × AI领域) → 所属领域>");
});

test("row connector creates two-branch groups and deleting at the minimum clears a branch with undo", async ({ page }) => {
  await page.goto("/nal-editor.html");
  await page.getByRole("button", { name: "从空白开始", exact: true }).click();
  await editSlot(page, "条件 1 连接符", "disjunction（∨）");
  const group = page.getByRole("group", { name: "分组 1", exact: true });
  await expect(group.getByRole("button", { name: /^条件 1\.\d 连接符$/ })).toHaveCount(2);
  await editSlot(page, "条件 1.1 连接符", "-->（继承）");
  await editSlot(page, "条件 1.1 左槽", "变量", "x");
  await editSlot(page, "条件 1.1 右槽", "AI领域");
  await page.getByRole("button", { name: "移除条件 1.1", exact: true }).click();
  await expect(page.getByLabel("条件 1.1 Narsese 语句", { exact: true })).toHaveText("[选择连接符]");
  await expect(group.getByRole("button", { name: /^条件 1\.\d 连接符$/ })).toHaveCount(2);
  await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
  await expect(page.getByLabel("条件 1.1 Narsese 语句", { exact: true })).toHaveText("?x → AI领域");
  await editSlot(page, "条件 1.2 连接符", "conjunction（∧）");
  await expect(page.getByRole("group", { name: "分组 1.2", exact: true }).getByRole("button", { name: /^条件 1\.2\.\d 连接符$/ })).toHaveCount(2);
  await page.screenshot({ path: ".tmp/verification/nal-two-branch-groups.png" });
  await page.getByRole("button", { name: "移除分组 1.2", exact: true }).click();
  await expect(group.getByRole("button", { name: /^条件 1\.\d 连接符$/ })).toHaveCount(2);
  await expect(page.getByLabel("条件 1.2 Narsese 语句", { exact: true })).toHaveText("[选择连接符]");
});

for (const route of ["/nal-editor.html", "/?preview=app"]) {
  test(`statement connector offers groups only while unset, including after clear and history restoration: ${route}`, async ({ page }) => {
    await page.goto(route);
    if (route === "/nal-editor.html") await page.getByRole("button", { name: "从空白开始", exact: true }).click();
    else await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
    const trigger = page.getByRole("button", { name: "条件 1 连接符", exact: true });
    const menu = page.getByRole("menu", { name: "条件 1 连接符", exact: true });
    const statementChoices = ["-->（继承）", "==>（蕴含）"];
    const allChoices = [...statementChoices, "conjunction（∧）", "disjunction（∨）"];
    await trigger.click();
    await expect(menu.getByRole("menuitemradio")).toHaveText(allChoices);
    await menu.getByRole("menuitemradio", { name: "-->（继承）", exact: true }).click();
    await trigger.click();
    // Even when both term slots remain empty, the chosen copula locks the row to a statement.
    await expect(menu.getByRole("menuitemradio")).toHaveText(statementChoices);
    await expect(menu.getByRole("menuitem", { name: "清空", exact: true })).toBeEnabled();
    await menu.getByRole("menuitemradio", { name: "==>（蕴含）", exact: true }).click();
    await trigger.click();
    await expect(menu.getByRole("menuitemradio")).toHaveText(statementChoices);
    if (route === "/nal-editor.html") await page.screenshot({ path: ".tmp/verification/nal-selected-copula-menu.png" });
    await menu.getByRole("menuitem", { name: "清空", exact: true }).click();
    await trigger.click();
    await expect(menu.getByRole("menuitemradio")).toHaveText(allChoices);
    if (route === "/nal-editor.html") await page.screenshot({ path: ".tmp/verification/nal-unset-copula-menu.png" });
    await menu.press("Escape");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await trigger.click();
    await expect(menu.getByRole("menuitemradio")).toHaveText(statementChoices);
    await menu.press("Escape");
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await trigger.click();
    await expect(menu.getByRole("menuitemradio")).toHaveText(allChoices);
    await menu.getByRole("menuitemradio", { name: "disjunction（∨）", exact: true }).click();
    await expect(page.getByRole("button", { name: "分组 1 connector", exact: true })).toHaveText("∨ disjunction");
  });
}

test("clearing a populated production query removes terms and outputs and cannot revive them by reselecting a copula", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  const submit = page.getByRole("button", { name: "查询文献", exact: true });
  for (const copula of ["-->（继承）", "==>（蕴含）"]) {
    await editSlot(page, "条件 1 连接符", copula);
    await editSlot(page, "条件 1 左槽", "变量", "文章");
    await editSlot(page, "条件 1 右槽", "AI领域");
    await expect(submit).toBeEnabled();
    const original = await expression.innerText();
    await page.getByRole("button", { name: "条件 1 连接符", exact: true }).click();
    await page.getByRole("menuitem", { name: "清空", exact: true }).click();
    await expect(submit).toBeDisabled();
    await expect(page.getByRole("button", { name: "移除输出变量 ?文章", exact: true })).toHaveCount(0);
    await editSlot(page, "条件 1 连接符", copula);
    await expect(expression).toHaveText(copula === "-->（继承）" ? "[选择词项] → [选择词项]" : "[选择词项] ⇒ [选择词项]");
    await expect(submit).toBeDisabled();
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(expression).toHaveText("[选择连接符]");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(expression).toHaveText(original);
    await expect(submit).toBeEnabled();
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await expect(submit).toBeDisabled();
  }
});

test("clearing either connector collapses a multi-slot compound to one empty term in the production query", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
  await editSlot(page, "条件 1 连接符", "-->（继承）");
  await editSlot(page, "条件 1 左槽", "product（×）");
  await editSlot(page, "条件 1 左槽 · 第 1 项", "变量", "文章");
  await editSlot(page, "条件 1 左槽 · 第 2 项", "AI领域");
  await editSlot(page, "条件 1 右槽", "所属领域");
  await page.getByRole("button", { name: "条件 1 左槽 connector 1", exact: true }).click();
  await page.getByRole("menuitem", { name: "增加槽位", exact: true }).click();
  const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
  for (const [index, option] of [[2, "conjunction（∧）"], [1, "disjunction（∨）"]] as const) {
    await editSlot(page, `条件 1 左槽 connector ${index}`, option);
    const before = await expression.textContent() ?? "";
    await page.getByRole("button", { name: `条件 1 左槽 connector ${index}`, exact: true }).click();
    await page.getByRole("menuitem", { name: "清空", exact: true }).click();
    await expect(expression).toHaveText("[选择词项] → 所属领域");
    await expect(expression.locator(".nal-compound-item")).toHaveCount(0);
    await expect(page.getByRole("button", { name: "条件 1 左槽", exact: true })).toBeFocused();
    await expect(page.getByRole("button", { name: "查询文献", exact: true })).toBeDisabled();
    if (index === 2) await page.screenshot({ path: ".tmp/verification/nal-cleared-compound-slot.png" });
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(expression).toHaveText(before);
    await expect(expression.locator(".nal-compound-item")).toHaveCount(3);
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await expect(expression).toHaveText("[选择词项] → 所属领域");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
  }
});

for (const route of ["/nal-editor.html", "/?preview=app"]) {
  test(`concept search marks relation terms without changing their names or selection: ${route}`, async ({ page }) => {
    await page.goto(route);
    if (route !== "/nal-editor.html") {
      await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
      await editSlot(page, "条件 1 连接符", "-->（继承）");
    }
    await page.getByRole("button", { name: "条件 1 右槽", exact: true }).click();
    await page.getByRole("menuitem", { name: "条件 1 右槽 atomic", exact: true }).click();
    await page.getByRole("menuitem", { name: "条件 1 右槽 concept", exact: true }).click();
    const menu = page.getByRole("menu", { name: "条件 1 右槽 concept", exact: true });
    for (const label of ["所属领域", "基于方法", "研究发展"]) {
      const item = menu.getByRole("menuitemradio", { name: label, exact: true });
      await expect(item.locator(".nal-menu-relation-badge")).toHaveText("关系");
      await expect(item).toHaveAccessibleDescription("关系词项");
    }
    await expect(menu.getByRole("menuitemradio", { name: "AI领域", exact: true }).locator(".nal-menu-relation-badge")).toHaveCount(0);
    await expect(menu.getByRole("menuitemradio", { name: "深度学习领域", exact: true }).locator(".nal-menu-relation-badge")).toHaveCount(0);
    if (route === "/nal-editor.html") await page.screenshot({ path: ".tmp/verification/nal-relation-term-badges.png" });
    else await page.screenshot({ path: ".tmp/verification/nal-relation-term-badges-production.png" });
    const search = menu.getByRole("searchbox", { name: "搜索概念", exact: true });
    await search.fill("基于");
    await expect(menu.getByRole("menuitemradio")).toHaveCount(1);
    await expect(menu.locator(".nal-menu-relation-badge")).toHaveCount(1);
    await search.press("Enter");
    await expect(page.getByRole("button", { name: "条件 1 右槽", exact: true })).toHaveText("基于方法");
    await expect(menu).toHaveCount(0);
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(page.getByRole("button", { name: "条件 1 右槽", exact: true })).toHaveText(route === "/nal-editor.html" ? "所属领域" : "[选择词项]");
  });
}

for (const route of ["/nal-editor.html", "/?preview=app"]) {
  test(`compound slot menus insert at the selected separator, enforce limits and isolate nested edits: ${route}`, async ({ page }) => {
    await page.goto(route);
    if (route !== "/nal-editor.html") {
      await page.getByRole("button", { name: "设置文献查询", exact: true }).click();
      await editSlot(page, "条件 1 连接符", "-->（继承）");
      await editSlot(page, "条件 1 左槽", "product（×）");
      await editSlot(page, "条件 1 左槽 · 第 1 项", "变量", "文章");
      await editSlot(page, "条件 1 左槽 · 第 2 项", "AI领域");
      await editSlot(page, "条件 1 右槽", "所属领域");
    }
    const expression = page.getByLabel("条件 1 Narsese 语句", { exact: true });
    const connector = "条件 1 左槽 connector 1";
    const add = async (label: string) => {
      await page.getByRole("button", { name: label, exact: true }).click();
      const menu = page.getByRole("menu", { name: label, exact: true });
      await expect(menu).toHaveClass(/note-action-menu/);
      await menu.getByRole("menuitem", { name: "增加槽位", exact: true }).click();
      await expect(menu).toHaveCount(0);
    };
    await add("条件 1 左槽 左括号");
    await expect(expression).toHaveText("([选择词项] × ?文章 × AI领域) → 所属领域");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await expect(expression).toHaveText("([选择词项] × ?文章 × AI领域) → 所属领域");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await add("条件 1 左槽 右括号");
    await expect(expression).toHaveText("(?文章 × AI领域 × [选择词项]) → 所属领域");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await add(connector);
    await expect(expression).toHaveText("(?文章 × [选择词项] × AI领域) → 所属领域");
    await editSlot(page, "条件 1 左槽 · 第 2 项", "conjunction（∧）");
    const nested = "条件 1 左槽 · 第 2 项 connector 1";
    await add(nested);
    await expect(expression).toHaveText("(?文章 × ([选择词项] ∧ [选择词项] ∧ [选择词项]) × AI领域) → 所属领域");
    // A compound has no bracket menu; its own connector hosts deletion of its parent slot.
    await expect(page.getByRole("button", { name: "条件 1 左槽 · 第 2 项", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: nested, exact: true }).click();
    const nestedMenu = page.getByRole("menu", { name: nested, exact: true });
    const deletion = nestedMenu.getByRole("menuitem", { name: "删除槽位", exact: true });
    await expect(deletion).toHaveAccessibleDescription("整个组合及内部词项");
    const caption = deletion.locator("small");
    await expect(caption).toHaveText("整个组合");
    const style = await caption.evaluate(element => ({ size: parseFloat(getComputedStyle(element).fontSize), labelSize: parseFloat(getComputedStyle(element.parentElement!).fontSize), color: getComputedStyle(element).color, labelColor: getComputedStyle(element.parentElement!).color }));
    expect(style.size).toBeLessThan(style.labelSize);
    expect(style.color).not.toBe(style.labelColor);
    await page.screenshot({ path: `.tmp/verification/nal-slot-menu-${route === "/nal-editor.html" ? "preview" : "production"}.png` });
    await nestedMenu.getByRole("menuitem", { name: "删除槽位", exact: true }).click();
    await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
    await page.getByRole("button", { name: "撤销语句编辑", exact: true }).click();
    await expect(expression).toContainText("∧");
    await page.getByRole("button", { name: "重做语句编辑", exact: true }).click();
    await expect(expression).toHaveText("(?文章 × AI领域) → 所属领域");
    for (let count = 2; count < 16; count++) await add(connector);
    await expect(expression.locator(".nal-compound-item")).toHaveCount(16);
    await page.getByRole("button", { name: connector, exact: true }).click();
    const fullMenu = page.getByRole("menu", { name: connector, exact: true });
    await expect(fullMenu.getByRole("menuitem", { name: "增加槽位", exact: true })).toBeDisabled();
    await fullMenu.press("Escape");
    for (const side of ["左括号", "右括号"]) {
      await page.getByRole("button", { name: `条件 1 左槽 ${side}`, exact: true }).click();
      const menu = page.getByRole("menu", { name: `条件 1 左槽 ${side}`, exact: true });
      await expect(menu.getByRole("menuitem", { name: "增加槽位", exact: true })).toBeDisabled();
      await menu.press("Escape");
    }
    await page.getByRole("button", { name: "条件 1 左槽 · 第 2 项", exact: true }).click();
    await page.getByRole("menuitem", { name: "删除槽位", exact: true }).click();
    await expect(expression.locator(".nal-compound-item")).toHaveCount(15);
    await page.getByRole("button", { name: connector, exact: true }).click();
    await expect(page.getByRole("menuitem", { name: "增加槽位", exact: true })).toBeEnabled();
  });
}
