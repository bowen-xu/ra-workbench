import {publicPackagePath} from "../ui/projectPaths";
import {expect,test} from "@playwright/test";
import {installNoteWorkspaceQueryMetrics,readNoteWorkspaceQueryMetrics,resetNoteWorkspaceQueryMetrics} from "./note-workspace-query-metrics.browser";
import {installWritingQueryTrace,readWritingQueryTrace} from "./writing-query-trace.browser";

for (const {bindingCount,imported} of [{bindingCount:0,imported:false},{bindingCount:1,imported:false},{bindingCount:12,imported:false},{bindingCount:0,imported:true}]) test(`65-source manuscript with ${imported ? "imports only" : `${bindingCount} bindings`} bounds initial projection reads and retains hot files`,async ({page}) => {
  test.setTimeout(90_000);
  const errors: string[] = [];page.on("pageerror",reason => errors.push(reason.message));
  await page.goto(imported ? "/?preview=app&bibliography=1" : "/?preview=app");await page.getByRole("button",{name:"写作",exact:true}).waitFor();
  // Seed through real adapters before the writing workspace is mounted. There
  // are no live editor/checkpoint owners during this committed fixture setup.
  const fixture = await page.evaluate(async paths => {
    const {getWritingGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/index.ts");
    const {getLiteratureGateway} = await import("/plugins/ra-literature-library/src/public/literatureGateway.ts");
    const {getAppStorage,PREVIEW_STORAGE_PREFIX} = await import("/ra-workbench/apps/web/src/core/runtime/appStorage.ts");
    const {storagePersistence} = await import("/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts");
    const {parseManuscriptPath,planCitationKeys} = await import("/@fs/" + paths.writing);
    const {exportLiterature} = await import("/@fs/" + paths.citation);
    const {emptyLiteratureDraft} = await import("/@fs/" + paths.literature);
    const gateway = getWritingGateway(), project = await gateway.createManuscript({title:"Stable key read bounds"});
    await gateway.saveFile({manuscriptId:project.id,relativePath:parseManuscriptPath("main.tex"),content:"\\section{Main source}\n"});
    for (let i = 0; i < 64; i++) await gateway.createFile(project.id,`chapter-${i}.tex`,
      `\\section{Chapter ${i}}\n` + (i === 31 && paths.bindingCount ? "\\cite{stable2026-0}\n" : "Synthetic source ".repeat(50)));
    for (let index = 0; index < paths.bindingCount; index++) {
      const key = `stable2026-${index}`;
      const item = await getLiteratureGateway().save({...emptyLiteratureDraft(),title:`Synthetic bound item ${index}`,bibtexKey:key});
      await gateway.bindCitation(project.id,{literatureItemId:item.id,requestedKey:key,bibtexEntry:exportLiterature([item],"bibtex")});
    }
    const all = await gateway.listBibliographyEntries(project.id,true);
    if (paths.imported && !all.length) throw Error("Synthetic imported bibliography is missing");
    if (all.length) await gateway.rekeyCitations(project.id,planCitationKeys(all,new Set(all.map(row => row.id))));
    await storagePersistence(getAppStorage()).flush();
    const prefix = paths.imported ? "research-assistant.bibliography-preview.v3:" : PREVIEW_STORAGE_PREFIX;
    return {names:{notes:"",writing:"research-assistant.preview." + encodeURIComponent(prefix)},sourceFiles:65,projectId:project.id};
  },{writing:publicPackagePath("@research-assistant/writing"),citation:publicPackagePath("@research-assistant/core/citation"),literature:publicPackagePath("@research-assistant/core/literature"),bindingCount,imported});
  await installNoteWorkspaceQueryMetrics(page,fixture.names);await page.reload();
  await page.getByRole("button",{name:"写作",exact:true}).waitFor();await installWritingQueryTrace(page);
  await page.getByRole("button",{name:"写作",exact:true}).click();
  await resetNoteWorkspaceQueryMetrics(page);
  await page.locator(".writing-project-link").filter({hasText:"Stable key read bounds"}).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toContainText("Main source");
  const outlineToggle = page.getByRole("button",{name:"文档大纲",exact:true});
  if (await outlineToggle.getAttribute("aria-expanded") === "false") await outlineToggle.click();
  const outline = page.getByLabel("当前文件大纲");
  await expect(outline).toContainText("Main source");
  await expect.poll(() => readWritingQueryTrace(page)).toMatchObject({completed:{syncBibliography:1},pending:0,failed:{}});
  await expect.poll(async () => (await readNoteWorkspaceQueryMetrics(page)).pending).toBe(0);
  const first = await readNoteWorkspaceQueryMetrics(page);
  const initialCalls = await readWritingQueryTrace(page);
  expect(first.rows["writing.writingBodies"] ?? 0).toBeLessThanOrEqual(bindingCount ? 130 : 65);
  // Opening each file once is permitted. Wait on real analysis output, not a
  // delay, so the following retained-switch measurement includes deferred work.
  const chapter = page.locator(".writing-file-row").getByRole("button",{name:"chapter-0.tex",exact:true});
  const main = page.getByRole("tab",{name:"main.tex",exact:true});
  await chapter.click();await expect(outline).toContainText("Chapter 0");
  await main.click();await expect(outline).toContainText("Main source");
  const original = await editor.elementHandle();
  await expect.poll(async () => (await readNoteWorkspaceQueryMetrics(page)).pending).toBe(0);
  await resetNoteWorkspaceQueryMetrics(page);
  for (let i = 0; i < 5; i++) {
    await page.getByRole("tab",{name:"chapter-0.tex",exact:true}).click();await expect(outline).toContainText("Chapter 0");
    await main.click();await expect(outline).toContainText("Main source");
    await page.getByRole("button",{name:"文献",exact:true}).click();
    await page.getByRole("button",{name:"写作",exact:true}).click();await expect(editor).toBeVisible();
  }
  await expect.poll(async () => (await readNoteWorkspaceQueryMetrics(page)).pending).toBe(0);
  const hot = await readNoteWorkspaceQueryMetrics(page);
  console.log("writing-citation-preflight",JSON.stringify({sourceFiles:fixture.sourceFiles,bindingCount,imported,first,initialCalls,hot}));
  expect(hot.rows["writing.writingBodies"] ?? 0).toBe(0);
  expect(await original?.evaluate(element => element.isConnected)).toBe(true);
  expect(errors).toEqual([]);
  if (bindingCount === 1) {
    // Reopening metadata is distinct from merely showing a retained workspace.
    // Return through the original file tab with unchanged source hashes and
    // confirm fresh library data reaches the generated bibliography.
    const sourceHashes = await page.evaluate(async id => {
      const {getWritingGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/index.ts");
      return (await getWritingGateway().listFiles(id)).filter(file => file.kind === "tex").map(file => [file.relativePath,file.sha256]);
    },fixture.projectId);
    await page.getByRole("tab",{name:"返回写作项目列表",exact:true}).click();
    await page.getByRole("button",{name:"新建稿件",exact:true}).click();
    await page.getByRole("dialog",{name:"新建 LaTeX 稿件"}).getByRole("textbox").fill("Other metadata scope");
    await page.getByRole("button",{name:"创建稿件",exact:true}).click();await expect(editor).toContainText("\\documentclass");
    await page.evaluate(async () => {
      const {getLiteratureGateway} = await import("/plugins/ra-literature-library/src/public/literatureGateway.ts");
      const gateway = getLiteratureGateway(), item = (await gateway.list()).find(row => row.title === "Synthetic bound item 0");
      if (!item) throw Error("Synthetic bound item is missing");
      await gateway.save({...item,title:"Fresh metadata after project return"});
    });
    await page.getByRole("tab",{name:"main.tex · Stable key read bounds",exact:true}).click();await expect(editor).toContainText("Main source");
    await page.locator(".writing-file-row").getByRole("button",{name:"references.bib",exact:true}).click();
    await expect(editor).toContainText("Fresh metadata after project return");
    expect(await page.evaluate(async id => {
      const {getWritingGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/index.ts");
      return (await getWritingGateway().listFiles(id)).filter(file => file.kind === "tex").map(file => [file.relativePath,file.sha256]);
    },fixture.projectId)).toEqual(sourceHashes);
    expect(errors).toEqual([]);
  }
  await page.screenshot({path:".tmp/writing-citation-preflight.png"});
});
