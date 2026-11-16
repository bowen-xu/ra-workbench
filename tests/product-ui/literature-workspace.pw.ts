import { expect, test, type Locator, type Page } from "@playwright/test";

const LITERATURE_STORAGE_KEY = "research-assistant.literature.v1";

function literatureItems(count = 48) {
  return Array.from({ length: count }, (_, index) => ({
    id: `paper-${index}`,
    itemType: "journalArticle",
    title: `Regression paper ${String(index + 1).padStart(2, "0")}`,
    abstractText: "A browser-only fixture used to verify the literature workspace layout.",
    containerTitle: "Interaction Testing",
    volume: "1",
    issue: "1",
    pages: `${index + 1}-${index + 9}`,
    doi: `10.0000/regression.${index}`,
    url: "",
    language: "en",
    bibtexKey: `regression${index}`,
    creators: [{ familyName: `Author ${index}`, givenName: "Test", literalName: "", role: "author" }],
    tags: ["layout", "regression"],
    readingStatus: "unread",
    attachments: index % 2 === 0 ? [{
      id: `attachment-${index}`,
      displayName: `regression-${index}.pdf`,
      mimeType: "application/pdf",
      byteSize: 1024,
      contentSha256: "a".repeat(64),
    }] : [],
    projectIds: [],
    metadataSources: [],
    createdAt: "2026-08-05T00:00:00.000Z",
    updatedAt: "2026-08-05T00:00:00.000Z",
  }));
}

async function openSeededLibrary(page: Page): Promise<void> {
  await page.addInitScript(({ key, items }) => {
    window.localStorage.clear();
    window.localStorage.setItem(key, JSON.stringify(items));
  }, { key: LITERATURE_STORAGE_KEY, items: literatureItems() });
  await page.goto("/");
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(page.locator(".library-workspace")).toBeVisible();
  await expect(page.locator(".literature-row")).toHaveCount(48);
}

async function expectFocusShellContract(input: Locator): Promise<void> {
  const initialStyle = await input.evaluate((element) => {
    const shell = element.closest<HTMLElement>("[data-focus-shell]");
    if (shell === null) throw new Error("compound input is missing its focus shell");
    const style = getComputedStyle(shell);
    return { borderColor: style.borderColor, boxShadow: style.boxShadow };
  });
  await input.focus();
  const focusStyle = await input.evaluate((element) => {
    const shell = element.closest<HTMLElement>("[data-focus-shell]");
    if (shell === null) throw new Error("compound input is missing its focus shell");
    const inputStyle = getComputedStyle(element);
    const shellStyle = getComputedStyle(shell);
    return {
      inputBoxShadow: inputStyle.boxShadow,
      inputOutlineStyle: inputStyle.outlineStyle,
      shellBorderColor: shellStyle.borderColor,
      shellBoxShadow: shellStyle.boxShadow,
    };
  });
  expect(focusStyle.inputOutlineStyle).toBe("none");
  expect(focusStyle.inputBoxShadow).toBe("none");
  expect(focusStyle.shellBorderColor).toBe(initialStyle.borderColor);
  expect(focusStyle.shellBoxShadow).toBe(initialStyle.boxShadow);
}

async function measurePdfPageGutters(page: Page) {
  await expect(page.locator(".pdf-view")).toBeVisible();
  return page.evaluate(() => {
    const layoutHost = document.createElement("div");
    layoutHost.className = "pdf-view pdf-viewer-container";
    Object.assign(layoutHost.style, {
      bottom: "auto",
      height: "480px",
      left: "-10000px",
      position: "fixed",
      right: "auto",
      top: "0",
      width: "640px",
    });
    const layoutViewer = document.createElement("div");
    layoutViewer.className = "pdfViewer";
    const pageFixture = document.createElement("div");
    pageFixture.className = "page";
    pageFixture.style.height = "900px";
    layoutViewer.append(pageFixture);
    layoutHost.append(layoutViewer);
    document.body.append(layoutHost);

    const horizontalGaps = () => {
      const viewerRect = layoutHost.getBoundingClientRect();
      const pageRect = pageFixture.getBoundingClientRect();
      return {
        left: pageRect.left - viewerRect.left + layoutHost.scrollLeft,
        right: layoutHost.scrollWidth - (pageRect.right - viewerRect.left + layoutHost.scrollLeft),
      };
    };
    const fittedPageWidth = Math.max(120, layoutHost.clientWidth - 76);
    pageFixture.style.width = `${fittedPageWidth}px`;
    const fittedGaps = horizontalGaps();

    pageFixture.style.width = `${layoutHost.clientWidth + 400}px`;
    layoutHost.scrollLeft = 0;
    const overflowLeftGap = horizontalGaps().left;
    layoutHost.scrollLeft = layoutHost.scrollWidth;
    const overflowMaxScrollLeft = layoutHost.scrollLeft;
    const overflowRightGap = horizontalGaps().right;

    pageFixture.style.width = `${fittedPageWidth}px`;
    const restoredFittedGaps = horizontalGaps();
    const restoredScrollLeft = layoutHost.scrollLeft;
    const layoutViewerRect = layoutViewer.getBoundingClientRect();
    const pageRect = pageFixture.getBoundingClientRect();
    const viewerStyle = getComputedStyle(layoutViewer);
    const result = {
      fittedGaps,
      overflowGaps: { left: overflowLeftGap, right: overflowRightGap },
      overflowMaxScrollLeft,
      pageBackground: getComputedStyle(pageFixture).backgroundColor,
      pageBoxShadow: getComputedStyle(pageFixture).boxShadow,
      pageTopGap: pageRect.top - layoutViewerRect.top,
      paddingLeft: viewerStyle.paddingLeft,
      paddingRight: viewerStyle.paddingRight,
      paddingTop: viewerStyle.paddingTop,
      restoredFittedGaps,
      restoredScrollLeft,
    };
    layoutHost.remove();
    return result;
  });
}

test("uses the compact application shell and a non-modal manual-entry popover", async ({ page }) => {
  await openSeededLibrary(page);

  const shell = page.locator(".app-shell");
  const sidebar = page.locator(".app-sidebar");
  const main = page.locator(".main-area");
  const updateButton = page.getByRole("button", { name: "检查更新" });
  const manualButton = page.getByRole("button", {name: "手动添加文献", exact: true});
  const libraryGrid = page.locator(".library-grid");
  const gridBefore = await libraryGrid.boundingBox();

  const shellMetrics = await shell.evaluate((element) => {
    const sidebarElement = element.querySelector<HTMLElement>(".app-sidebar");
    const mainElement = element.querySelector<HTMLElement>(".main-area");
    if (sidebarElement === null || mainElement === null) throw new Error("application shell is incomplete");
    const sidebarRect = sidebarElement.getBoundingClientRect();
    const mainRect = mainElement.getBoundingClientRect();
    const mainStyle = getComputedStyle(mainElement);
    return {
      documentClientHeight: document.documentElement.clientHeight,
      documentScrollHeight: document.documentElement.scrollHeight,
      mainLeft: mainRect.left,
      mainPaddingLeft: mainStyle.paddingLeft,
      mainPaddingRight: mainStyle.paddingRight,
      sidebarBottom: sidebarRect.bottom,
      sidebarRight: sidebarRect.right,
      sidebarTop: sidebarRect.top,
      viewportHeight: window.innerHeight,
      viewportWidth: window.innerWidth,
      shellWidth: element.getBoundingClientRect().width,
    };
  });
  expect(shellMetrics.documentScrollHeight).toBe(shellMetrics.documentClientHeight);
  expect(shellMetrics.sidebarTop).toBe(36);
  expect(shellMetrics.sidebarBottom).toBe(shellMetrics.viewportHeight);
  expect(shellMetrics.mainLeft).toBe(shellMetrics.sidebarRight);
  expect(shellMetrics.shellWidth).toBe(shellMetrics.viewportWidth);
  expect(shellMetrics.mainPaddingLeft).toBe("0px");
  expect(shellMetrics.mainPaddingRight).toBe("0px");
  await expect(main.locator(":scope > .topbar")).toHaveCount(0);

  const updateMetrics = await updateButton.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    const footer = element.closest(".sidebar-footer")?.getBoundingClientRect();
    return { borderRadius: getComputedStyle(element).borderRadius, height: rect.height, width: rect.width, footerBottom: footer?.bottom };
  });
  expect(updateMetrics.width).toBe(updateMetrics.height);
  expect(updateMetrics.width).toBe(30);
  expect(updateMetrics.borderRadius).toBe("50%");
  expect(updateMetrics.footerBottom).toBe(shellMetrics.viewportHeight - 20);

  const manualMetrics = await manualButton.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return { height: rect.height, svgCount: element.querySelectorAll("svg").length, width: rect.width };
  });
  expect(manualMetrics).toEqual({ height: 30, svgCount: 1, width: 30 });

  await manualButton.click();
  const popover = page.getByRole("dialog", { name: "手动添加文献" });
  await expect(popover).toBeVisible();
  await expect(popover.getByLabel("条目类型")).toBeVisible();
  await popover.getByPlaceholder("标题（必填）").fill("Unsubmitted draft");
  expect(await popover.evaluate((element) => getComputedStyle(element).position)).toBe("fixed");
  expect(await libraryGrid.boundingBox()).toEqual(gridBefore);

  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(popover).toBeHidden();
  await manualButton.click();
  await expect(page.getByPlaceholder("标题（必填）")).toHaveValue("Unsubmitted draft");
  expect(await page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? "[]").length, LITERATURE_STORAGE_KEY)).toBe(48);

  await page.getByRole("button", { name: "取消", exact: true }).click();
  await manualButton.click();
  await expect(page.getByPlaceholder("标题（必填）")).toHaveValue("");
  await page.getByPlaceholder("标题（必填）").fill("Escape draft");
  await page.keyboard.press("Escape");
  await expect(popover).toBeHidden();
  await expect(manualButton).toBeFocused();
  await manualButton.click();
  await expect(page.getByPlaceholder("标题（必填）")).toHaveValue("");
  await expect(sidebar).toBeVisible();
});

test("keeps structured-reading model configuration inside the narrow detail pane", async ({ page }) => {
  await openSeededLibrary(page);
  await page.locator(".literature-row").first().click();
  const panel = page.locator(".structured-reading-panel");
  await expect(panel).toBeVisible();
  await page.getByRole("button", { name: "辅助阅读模型设置" }).click();
  const popover = page.getByRole("dialog", { name: "辅助阅读模型设置" });
  await expect(popover).toBeVisible();

  const bounds = await panel.evaluate((element) => {
    const popoverElement = document.querySelector<HTMLElement>(".reading-settings-model");
    if (popoverElement === null) throw new Error("model configuration popover is missing");
    const panelRect = element.getBoundingClientRect();
    const popoverRect = popoverElement.getBoundingClientRect();
    const controls = [...popoverElement.querySelectorAll<HTMLElement>("input, select, button")];
    return {
      panelLeft: panelRect.left,
      panelRight: panelRect.right,
      popoverLeft: popoverRect.left,
      popoverRight: popoverRect.right,
      hasOverflowingControl: controls.some((control) => {
        const rect = control.getBoundingClientRect();
        return rect.left < popoverRect.left || rect.right > popoverRect.right;
      }),
      horizontallyScrollable: popoverElement.scrollWidth > popoverElement.clientWidth,
    };
  });
  expect(bounds.popoverLeft).toBeGreaterThanOrEqual(0);
  expect(bounds.popoverRight).toBeLessThanOrEqual(await page.evaluate(() => innerWidth));
  expect(bounds.hasOverflowingControl).toBe(false);
  expect(bounds.horizontallyScrollable).toBe(false);
});

test("preserves the Zotero-like table, search, selection, and context-menu contract", async ({ page }) => {
  await openSeededLibrary(page);

  const rows = page.locator(".literature-row");
  const firstRow = rows.filter({ hasText: "Regression paper 01" });
  const secondRow = rows.filter({ hasText: "Regression paper 02" });
  const thirdRow = rows.filter({ hasText: "Regression paper 03" });
  const fifthRow = rows.filter({ hasText: "Regression paper 05" });
  const search = page.getByRole("textbox", { name: "搜索文献" });

  await expect(page.locator(".literature-row.selected")).toHaveCount(0);
  await expect(page.locator(".detail-empty")).toHaveText("选择一篇文献查看详情");
  await expect(page.locator(".library-toolbar select")).toHaveCount(0);
  await expect(page.getByText(/\d+\s*\/\s*\d+\s*篇文献/)).toHaveCount(0);
  await expect(page.getByText("本地资料库", { exact: true })).toHaveCount(0);
  await expect(page.getByText("导出当前项目", { exact: true })).toHaveCount(0);
  await expect(page.getByText("批量导出", { exact: true })).toHaveCount(0);

  const visualContract = await page.evaluate(() => {
    const header = document.querySelector<HTMLElement>(".literature-columns");
    const titleHeader = document.querySelector<HTMLElement>(".literature-column.title button span");
    const firstRow = document.querySelector<HTMLElement>(".literature-row");
    const title = firstRow?.querySelector<HTMLElement>(".literature-cell.title strong");
    const creator = firstRow?.querySelector<HTMLElement>(".row-creator");
    const lastCell = firstRow?.querySelector<HTMLElement>(".literature-cell:last-child");
    if (header === null || titleHeader === null || firstRow === null || title === null || creator === null || lastCell === null) {
      throw new Error("literature table is incomplete");
    }
    const headerRect = header.getBoundingClientRect();
    const rowRect = firstRow.getBoundingClientRect();
    const lastCellRect = lastCell.getBoundingClientRect();
    return {
      creatorFontSize: getComputedStyle(creator).fontSize,
      headerRight: headerRect.right,
      lastCellRight: lastCellRect.right,
      rowRight: rowRect.right,
      titleFontSize: getComputedStyle(title).fontSize,
      titleHeaderX: titleHeader.getBoundingClientRect().x,
      titleX: title.getBoundingClientRect().x,
    };
  });
  expect(visualContract.titleFontSize).toBe("13px");
  expect(visualContract.creatorFontSize).toBe("12px");
  expect(Math.abs(visualContract.titleHeaderX - visualContract.titleX)).toBeLessThanOrEqual(1);
  expect(Math.abs(visualContract.headerRight - visualContract.rowRight)).toBeLessThanOrEqual(1);
  expect(Math.abs(visualContract.lastCellRight - visualContract.rowRight)).toBeLessThanOrEqual(1);

  const attachmentHeader = page.locator(".literature-column.attachment > button");
  await expect(attachmentHeader).toHaveText("");
  await expect(attachmentHeader.locator("svg")).toHaveCount(1);
  await expect(firstRow.locator(".attachment-file-icon")).toHaveCount(1);
  await expect(secondRow.locator(".attachment-file-icon")).toHaveCount(0);
  expect(await firstRow.locator(".attachment-file-mark").evaluate((element) => getComputedStyle(element).stroke))
    .toBe("rgb(196, 77, 61)");

  await firstRow.click();
  await thirdRow.click({ modifiers: ["Meta"] });
  await expect(page.locator(".literature-row.selected")).toHaveCount(2);
  await fifthRow.click({ modifiers: ["Shift"] });
  await expect(page.locator(".literature-row.selected")).toHaveCount(3);
  expect(await fifthRow.locator(".attachment-file-mark").evaluate((element) => getComputedStyle(element).stroke))
    .toBe("rgb(255, 255, 255)");

  await search.fill("REGRESSION PAPER 02");
  await expect(rows).toHaveCount(1);
  await expect(secondRow).toBeVisible();
  await expect(page.locator(".literature-row.selected")).toHaveCount(0);
  await expect(page.locator(".detail-empty")).toHaveText("选择一篇文献查看详情");
  await search.fill("10.0000/regression.1");
  await expect(rows).toHaveCount(0);
  await search.fill("");
  await expect(rows).toHaveCount(48);

  await expectFocusShellContract(search);

  await page.locator(".literature-columns").click({ button: "right", position: { x: 220, y: 16 } });
  const columnMenu = page.getByRole("menu", { name: "显示列" });
  await expect(columnMenu).toBeVisible();
  await expect(columnMenu.getByRole("menuitemcheckbox", { name: "条目类型" })).toBeVisible();
  await expect(columnMenu.getByText("类型图标", { exact: true })).toHaveCount(0);
  await columnMenu.getByRole("menuitemcheckbox", { name: "作者" }).click();
  await expect(columnMenu).toBeHidden();
  await expect(page.locator(".literature-column.creator")).toHaveCount(0);

  await firstRow.click({ button: "right" });
  const itemMenu = page.getByRole("menu", { name: "文献操作" });
  await expect(itemMenu).toBeVisible();
  await expect(itemMenu.getByRole("menuitem", { name: "在 Finder 中显示", exact: true })).toBeVisible();
  await expect(itemMenu.getByRole("menuitem", { name: "导出所选为 BibTeX" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(itemMenu).toBeHidden();

  await firstRow.click();
  await page.getByRole("button", {name:"展开 Regression paper 01 的附件",exact:true}).click();
  await page.getByRole("button", {name:"打开附件 regression-0.pdf",exact:true}).click({ button: "right" });
  const attachmentMenu = page.getByRole("menu", { name: "附件操作" });
  await expect(attachmentMenu).toBeVisible();
  await expect(attachmentMenu.getByRole("menuitem", { name: "在 Finder 中显示" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(attachmentMenu).toBeHidden();

  const contextPolicy = await page.evaluate(() => {
    const chrome = document.querySelector<HTMLElement>(".literature-row");
    const input = document.querySelector<HTMLElement>(".library-search input");
    if (chrome === null || input === null) throw new Error("selection policy fixtures are missing");
    const chromeContext = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    const chromeSelection = new Event("selectstart", { bubbles: true, cancelable: true });
    const inputSelection = new Event("selectstart", { bubbles: true, cancelable: true });
    chrome.dispatchEvent(chromeContext);
    chrome.dispatchEvent(chromeSelection);
    input.dispatchEvent(inputSelection);
    return {
      chromeContextPrevented: chromeContext.defaultPrevented,
      chromeSelectionPrevented: chromeSelection.defaultPrevented,
      chromeUserSelect: getComputedStyle(chrome).userSelect,
      inputSelectionPrevented: inputSelection.defaultPrevented,
      inputUserSelect: getComputedStyle(input).userSelect,
    };
  });
  expect(contextPolicy).toEqual({
    chromeContextPrevented: true,
    chromeSelectionPrevented: true,
    chromeUserSelect: "none",
    inputSelectionPrevented: false,
    inputUserSelect: "text",
  });

  await secondRow.dblclick();
  const toast = page.locator("[data-sonner-toast]").filter({ hasText: "没有可在当前阅读器中打开的附件" });
  await expect(toast).toBeVisible();
  await expect(toast).toHaveClass(/app-toast/);
  await expect(page.locator(".notice-banner")).toHaveCount(0);

  await firstRow.dblclick();
  await expect(page.getByRole("tab", { name: /Regression paper 01/ })).toHaveAttribute("aria-selected", "true");
});

test("keeps PDF find in a floating popover without changing reader geometry", async ({ page }) => {
  await openSeededLibrary(page);
  await expectFocusShellContract(page.getByRole("textbox", { name: "搜索文献" }));

  await page.locator(".literature-row").filter({ hasText: "Regression paper 01" }).click();
  await page.getByRole("button", {name:"展开 Regression paper 01 的附件",exact:true}).click();
  await page.getByRole("button", {name:"打开附件 regression-0.pdf",exact:true}).click();
  await expect(page.locator(".reader-workspace")).toHaveAttribute("data-reader-lifecycle", "active");
  const readerBody = page.locator(".reader-body");
  const toolbar = page.locator(".reader-toolbar");
  const bodyBefore = await readerBody.boundingBox();
  const toolbarBefore = await toolbar.boundingBox();
  await page.getByRole("button", { name: "在文档中查找", exact: true }).click();

  const readerSearch = page.getByRole("textbox", { name: "在文档中查找" });
  await expect(readerSearch).toBeFocused();
  await expectFocusShellContract(readerSearch);
  const findPopover = page.getByRole("search", { name: "文档查找" });
  const toolbarBox = await toolbar.boundingBox();
  const bodyWithFind = await readerBody.boundingBox();
  const popoverBox = await findPopover.boundingBox();
  if (bodyBefore === null || toolbarBefore === null || toolbarBox === null || bodyWithFind === null || popoverBox === null) throw new Error("find popover layout is unavailable");
  expect(toolbarBox).toEqual(toolbarBefore);
  expect(bodyWithFind).toEqual(bodyBefore);
  expect(popoverBox.y).toBeGreaterThanOrEqual(toolbarBox.y + toolbarBox.height);
  expect(await findPopover.evaluate((element) => {
    const host = element.closest("[data-overlay-portal]");
    return host?.getAttribute("data-overlay-portal");
  })).toBe("popup");
  await expect(findPopover.getByText("全部高亮", { exact: true })).toBeVisible();
  await expect(findPopover.getByText("区分大小写", { exact: true })).toBeVisible();
  await expect(findPopover.getByText("全词匹配", { exact: true })).toBeVisible();

  await page.keyboard.press("Escape");
  await expect(findPopover).toBeHidden();
  expect(await toolbar.boundingBox()).toEqual(toolbarBefore);
  expect(await readerBody.boundingBox()).toEqual(bodyBefore);
  await page.getByRole("tab", {name:"高亮与笔记",exact:true}).focus();
  await page.keyboard.press("Meta+f");
  await expect(readerSearch).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(findPopover).toBeHidden();
  await page.getByRole("tab", { name: /Regression paper 01/ }).focus();
  await page.keyboard.press("Meta+f");
  await expect(readerSearch).toBeFocused();
});

test("preserves symmetric PDF page gutters at fitted and overflow widths", async ({ page }) => {
  await openSeededLibrary(page);
  await page.locator(".literature-row").filter({ hasText: "Regression paper 01" }).click();
  await page.getByRole("button", {name:"展开 Regression paper 01 的附件",exact:true}).click();
  await page.getByRole("button", {name:"打开附件 regression-0.pdf",exact:true}).click();

  const gutters = await measurePdfPageGutters(page);
  expect(gutters.paddingLeft).toBe("18px");
  expect(gutters.paddingRight).toBe("18px");
  expect(gutters.paddingTop).toBe("12px");
  expect(gutters.fittedGaps.left).toBeGreaterThan(0);
  expect(Math.abs(gutters.fittedGaps.left - gutters.fittedGaps.right)).toBeLessThanOrEqual(1);
  expect(gutters.overflowMaxScrollLeft).toBeGreaterThan(0);
  expect(gutters.overflowGaps.left).toBeGreaterThanOrEqual(18);
  expect(gutters.overflowGaps.right).toBeGreaterThanOrEqual(18);
  expect(Math.abs(gutters.overflowGaps.left - gutters.overflowGaps.right)).toBeLessThanOrEqual(1);
  expect(gutters.restoredScrollLeft).toBe(0);
  expect(Math.abs(gutters.restoredFittedGaps.left - gutters.restoredFittedGaps.right)).toBeLessThanOrEqual(1);
  expect(gutters.pageTopGap).toBeGreaterThanOrEqual(12);
  expect(gutters.pageBackground).toBe("rgb(255, 255, 255)");
  expect(gutters.pageBoxShadow).not.toBe("none");
});

test("shows transient import feedback and opens PDFs in reusable workspace tabs", async ({ page }) => {
  await openSeededLibrary(page);
  const libraryGrid = page.locator(".library-grid");
  const gridBefore = await libraryGrid.boundingBox();

  await page.getByRole("button", {name: "从 BibTeX 添加", exact: true}).click();
  const bibtexDialog = page.getByRole("dialog", { name: "从 BibTeX 添加文献" });
  await bibtexDialog.locator("textarea").fill("@article{toast_test, title={Toast regression}, author={Tester, Tina}, year={2026}}");
  await bibtexDialog.getByRole("button", { name: "检查并导入" }).click();

  const importToast = page.locator(".import-toast");
  await expect(importToast).toContainText("已导入 1 篇文献");
  expect(await importToast.evaluate((element) => {
    const toaster = element.closest("[data-sonner-toaster]");
    return toaster === null ? "" : getComputedStyle(toaster).position;
  })).toBe("fixed");
  expect(await libraryGrid.boundingBox()).toEqual(gridBefore);
  await expect(importToast).toBeHidden({ timeout: 6_000 });

  await page.getByRole("button", {name: "从 BibTeX 添加", exact: true}).click();
  await page.getByRole("dialog", { name: "从 BibTeX 添加文献" }).locator("textarea")
    .fill("@article{close_test, title={Close regression}, author={Tester, Tina}, year={2026}}");
  await page.getByRole("dialog", { name: "从 BibTeX 添加文献" })
    .getByRole("button", { name: "检查并导入" }).click();
  await expect(importToast).toBeVisible();
  await importToast.getByRole("button", { name: "关闭通知" }).click();
  await expect(importToast).toBeHidden();

  await page.locator(".literature-row").filter({ hasText: "Regression paper 01" }).click();
  await page.locator(".literature-row").filter({hasText:"Regression paper 01"}).dblclick();
  const readerTab = page.getByRole("tab", { name: /Regression paper 01/ });
  await expect(readerTab).toBeVisible();
  await expect(readerTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".reader-workspace")).toHaveAttribute("data-reader-lifecycle", "active");

  await page.getByRole("tab", { name: "返回文献列表", exact: true }).click();
  await expect(readerTab).toHaveAttribute("aria-selected", "false");
  await page.locator(".literature-row").filter({hasText:"Regression paper 01"}).dblclick();
  await expect(readerTab).toHaveCount(1);
  await expect(readerTab).toHaveAttribute("aria-selected", "true");
  await expect(page.locator(".reader-workspace")).toHaveAttribute("data-reader-lifecycle", "active");

  await expect(page.getByRole("button", { name: "关闭阅读器" })).toHaveCount(0);
  await page.getByRole("button", { name: "关闭 Regression paper 01", exact: true }).click();
  await expect(readerTab).toHaveCount(0);
  await expect(page.locator(".library-surface")).toBeVisible();
});

test("keeps the library and reader shells fixed while their panes scroll independently", async ({ page }) => {
  await openSeededLibrary(page);

  await page.locator(".literature-row").filter({ hasText: "Regression paper 01" }).click();
  await page.locator(".literature-detail").evaluate((element) => {
    const filler = document.createElement("div");
    filler.dataset.testFiller = "detail";
    filler.style.height = "900px";
    element.append(filler);
  });

  const libraryState = await page.evaluate(() => {
    const list = document.querySelector<HTMLElement>(".literature-table-scroll");
    const detail = document.querySelector<HTMLElement>(".literature-detail");
    const main = document.querySelector<HTMLElement>(".main-area");
    const sidebar = document.querySelector<HTMLElement>(".app-sidebar");
    const header = document.querySelector<HTMLElement>(".literature-columns");
    if (list === null || detail === null || main === null || sidebar === null || header === null) throw new Error("library panes are incomplete");
    list.scrollTop = 260;
    detail.scrollTop = 180;
    return {
      detailScrollTop: detail.scrollTop,
      headerTop: header.getBoundingClientRect().top,
      listScrollTop: list.scrollTop,
      listTop: list.getBoundingClientRect().top,
      mainScrollTop: main.scrollTop,
      sidebarTop: sidebar.getBoundingClientRect().top,
      windowScrollY: window.scrollY,
    };
  });
  expect(libraryState.listScrollTop).toBeGreaterThan(0);
  expect(libraryState.detailScrollTop).toBeGreaterThan(0);
  expect(libraryState.mainScrollTop).toBe(0);
  expect(libraryState.windowScrollY).toBe(0);
  expect(libraryState.sidebarTop).toBe(36);
  expect(Math.abs(libraryState.headerTop - libraryState.listTop)).toBeLessThanOrEqual(1);

  await page.locator(".literature-row").filter({hasText:"Regression paper 01"}).dblclick();
  await expect(page.locator(".reader-workspace")).toBeVisible();
  await expect(page.locator(".pdf-viewer-container")).toBeVisible();
  const toolbarContract = await page.locator(".reader-toolbar").evaluate((toolbar) => {
    const rect = toolbar.getBoundingClientRect();
    const buttonRects = [...toolbar.querySelectorAll("button")].map((button) => {
      const buttonRect = button.getBoundingClientRect();
      return {
        compactText: button.matches(".reader-structured-toggle"),
        height: buttonRect.height,
        width: buttonRect.width,
      };
    });
    return {
      buttonRects,
      closeButtons: toolbar.querySelectorAll('[aria-label="关闭阅读器"]').length,
      height: rect.height,
      titleBlocks: toolbar.querySelectorAll(".reader-title").length,
    };
  });
  expect(toolbarContract.height).toBe(41);
  expect(toolbarContract.buttonRects.length).toBeGreaterThan(0);
  expect(toolbarContract.buttonRects.every(({ compactText, height, width }) =>
    height === 28 && (compactText ? width >= 28 : width === 28))).toBe(true);
  expect(toolbarContract.titleBlocks).toBe(0);
  expect(toolbarContract.closeButtons).toBe(0);

  const stageWithBothSidebars = await page.locator(".pdf-stage").boundingBox();
  await page.getByRole("button", { name: "隐藏导航侧栏" }).click();
  await expect(page.locator(".reader-navigation")).toHaveCount(0);
  await page.getByRole("button", { name: "隐藏笔记侧栏" }).click();
  await expect(page.locator(".reader-notes")).toBeHidden();
  const stageWithoutSidebars = await page.locator(".pdf-stage").boundingBox();
  expect(stageWithBothSidebars).not.toBeNull();
  expect(stageWithoutSidebars).not.toBeNull();
  expect(stageWithoutSidebars?.width).toBeGreaterThan(stageWithBothSidebars?.width ?? 0);
  await page.getByRole("button", { name: "显示导航侧栏" }).click();
  await page.getByRole("button", { name: "显示笔记侧栏" }).click();
  await expect(page.locator(".reader-navigation")).toBeVisible();
  await expect(page.locator(".reader-notes")).toBeVisible();

  const navigationResizeHandle = page.getByRole("separator", { name: "调整导航侧栏宽度" });
  const resizeBox = await navigationResizeHandle.boundingBox();
  if (resizeBox === null) throw new Error("reader navigation resize handle is missing");
  await page.mouse.move(resizeBox.x + resizeBox.width / 2, resizeBox.y + 40);
  await page.mouse.down();
  await page.mouse.move(resizeBox.x + resizeBox.width / 2 + 36, resizeBox.y + 40, { steps: 4 });
  await page.mouse.up();
  const readerPreferences = await page.evaluate(() => JSON.parse(
    localStorage.getItem("research-assistant.reader-ui-session.v1:attachment-0") ?? "null",
  ));
  expect(readerPreferences).toMatchObject({
    schemaVersion: 1,
    navigationSidebar: { open: true, view: "thumbnails" },
    notesSidebar: { open: true, width: 300 },
  });
  expect(readerPreferences.navigationSidebar.width).toBeGreaterThan(190);

  const readerState = await page.evaluate(() => {
    const viewer = document.querySelector<HTMLElement>(".pdf-viewer-container");
    const navigation = document.querySelector<HTMLElement>(".reader-navigation");
    const notes = document.querySelector<HTMLElement>(".reader-sidebar-page:not([hidden])");
    const main = document.querySelector<HTMLElement>(".main-area");
    const sidebar = document.querySelector<HTMLElement>(".app-sidebar");
    const stage = document.querySelector<HTMLElement>(".pdf-stage");
    const toolbar = document.querySelector<HTMLElement>(".reader-toolbar");
    const pdfViewer = viewer?.querySelector<HTMLElement>(".pdfViewer");
    if (viewer === null || navigation === null || notes === null || main === null || sidebar === null || stage === null || toolbar === null || pdfViewer === null) throw new Error("reader panes are incomplete");
    const scrollFixture = document.createElement("div");
    scrollFixture.style.flex = "0 0 1800px";
    scrollFixture.style.width = "1px";
    pdfViewer.append(scrollFixture);
    for (const pane of [navigation, notes]) {
      const filler = document.createElement("div");
      filler.style.height = "1000px";
      pane.append(filler);
    }
    const toolbarTop = toolbar.getBoundingClientRect().top;
    viewer.scrollTop = 320;
    navigation.scrollTop = 140;
    notes.scrollTop = 170;
    const sidebarStyle = getComputedStyle(sidebar);
    return {
      mainScrollTop: main.scrollTop,
      navigationScrollTop: navigation.scrollTop,
      notesScrollTop: notes.scrollTop,
      readerBackground: getComputedStyle(stage).backgroundColor,
      sidebarBackground: sidebarStyle.backgroundColor,
      sidebarBorderRadius: sidebarStyle.borderRadius,
      sidebarBoxShadow: sidebarStyle.boxShadow,
      sidebarTop: sidebar.getBoundingClientRect().top,
      toolbarTop,
      toolbarTopAfterScroll: toolbar.getBoundingClientRect().top,
      viewerScrollTop: viewer.scrollTop,
      windowScrollY: window.scrollY,
    };
  });
  expect(readerState.viewerScrollTop).toBeGreaterThan(0);
  expect(readerState.navigationScrollTop).toBeGreaterThan(0);
  expect(readerState.notesScrollTop).toBeGreaterThan(0);
  expect(readerState.mainScrollTop).toBe(0);
  expect(readerState.windowScrollY).toBe(0);
  expect(readerState.readerBackground).toBe("rgb(217, 220, 218)");
  expect(readerState.sidebarBackground).toBe("rgb(31, 42, 36)");
  expect(readerState.sidebarBorderRadius).toBe("0px");
  expect(readerState.sidebarBoxShadow).toBe("none");
  expect(readerState.sidebarTop).toBe(36);
  expect(readerState.toolbarTopAfterScroll).toBe(readerState.toolbarTop);
});
