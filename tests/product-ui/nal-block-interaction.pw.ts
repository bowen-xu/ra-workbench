import {expect, test, type Locator, type Page} from "@playwright/test";
import {addBlock, editSlot} from "./nal-query-editor";

async function dragToBoundary(page:Page, handle:Locator, target:Locator):Promise<void> {
  const source=await handle.boundingBox(), row=await target.boundingBox();
  if(!source || !row)throw new Error("Missing shared block geometry");
  await page.mouse.move(source.x+source.width/2,source.y+source.height/2);
  await page.mouse.down();
  await page.mouse.move(row.x+4,row.y+row.height-2,{steps:12});
}

test("NAL uses shared block controls, pointer feedback and motion; cancel has no history and drop retains DOM identity",async({page})=>{
  await page.goto("/nal-editor.html");
  await addBlock(page);await editSlot(page,"条件 2 连接符","==>（蕴含）");
  const first=page.getByRole("group",{name:"条件 1",exact:true}), second=page.getByRole("group",{name:"条件 2",exact:true});
  const handle=page.getByRole("button",{name:"拖动条件 1",exact:true});
  await expect(handle).toHaveClass(/block-drag-button/);await expect(handle).toHaveAttribute("draggable","false");
  const plus=page.getByRole("button",{name:"添加 block：条件 1",exact:true});
  await expect(plus).toHaveClass(/block-add-button/);await expect(plus).toHaveCSS("width","24px");
  const sourceId=await first.getAttribute("data-nal-block-id");
  await first.evaluate(element=>element.setAttribute("data-test-retained-row","yes"));
  await page.evaluate(()=>{
    document.documentElement.dataset.nativeNalDrags="0";
    document.addEventListener("dragstart",()=>{document.documentElement.dataset.nativeNalDrags=String(Number(document.documentElement.dataset.nativeNalDrags)+1);});
  });
  await dragToBoundary(page,handle,second);
  await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
  await expect(page.locator(".note-block-pointer-preview")).toHaveText("移动块");
  await expect(first).toHaveCSS("opacity","0.35");
  await expect(second).not.toHaveCSS("transform","none");
  await page.screenshot({path:".tmp/verification/nal-shared-block-drag.png"});
  await page.keyboard.press("Escape");await page.mouse.up();
  await expect(page.locator(".note-block-drop-cursor")).toHaveCount(0);
  await expect(page.locator("style[data-block-reorder-preview]")).toHaveCount(0);
  await expect(page.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("(?文章 × AI领域) → 所属领域");
  await dragToBoundary(page,handle,second);await page.mouse.up();
  await expect(page.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("[选择词项] ⇒ [选择词项]");
  await expect(page.getByRole("group",{name:"条件 2",exact:true})).toHaveAttribute("data-nal-block-id",sourceId!);
  await expect(page.getByRole("group",{name:"条件 2",exact:true})).toHaveAttribute("data-test-retained-row","yes");
  await page.getByRole("button",{name:"撤销语句编辑",exact:true}).click();
  await expect(page.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("(?文章 × AI领域) → 所属领域");
  await page.getByRole("button",{name:"重做语句编辑",exact:true}).click();
  await expect(page.getByLabel("条件 2 Narsese 语句",{exact:true})).toHaveText("(?文章 × AI领域) → 所属领域");
  await expect(page.locator("html")).toHaveAttribute("data-native-nal-drags","0");
});

test("shared drag rejects NAL cross-parent destinations and respects reduced motion",async({page})=>{
  await page.emulateMedia({reducedMotion:"reduce"});
  await page.goto("/nal-editor.html");await page.getByRole("button",{name:"嵌套分组",exact:true}).click();
  const before=await page.locator(".nal-query-builder").textContent();
  const handle=page.getByRole("button",{name:"拖动条件 1.1",exact:true});
  const target=page.getByRole("group",{name:"条件 1.2.1",exact:true});
  await dragToBoundary(page,handle,target);
  await expect(page.locator(".note-block-pointer-preview")).toBeVisible();
  await expect(page.locator(".note-block-drop-cursor")).toBeHidden();
  await expect(page.locator(".nal-query-builder")).toHaveCSS("--reorder-motion-duration","0ms");
  await page.mouse.up();
  await expect(page.locator(".nal-query-builder")).toHaveText(before!);
  await expect(page.getByRole("button",{name:"撤销语句编辑",exact:true})).toBeDisabled();
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
});

test("shared block feedback paints and commits inside the saved-query native dialog",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  for(const [number,label] of [[1,"AI领域"],[2,"深度学习领域"]] as const){
    if(number===2)await addBlock(page);
    await editSlot(page,`条件 ${number} 连接符`,"-->（继承）");
    await editSlot(page,`条件 ${number} 左槽`,"变量","文章");
    await editSlot(page,`条件 ${number} 右槽`,label);
  }
  await page.getByRole("switch",{name:"保存为查询集合",exact:true}).check();
  await page.getByLabel("查询集合名称",{exact:true}).fill("共享块拖动");
  await page.getByRole("button",{name:"查询并保存",exact:true}).click();
  await page.getByRole("button",{name:"查询操作：共享块拖动",exact:true}).click();
  await page.getByRole("menuitem",{name:"编辑查询：共享块拖动",exact:true}).click();
  const dialog=page.getByRole("dialog",{name:"编辑查询集合",exact:true});
  await expect(dialog.getByRole("button",{name:"条件 1 连接符",exact:true})).toHaveCSS("border-width","0px");
  await dragToBoundary(page,dialog.getByRole("button",{name:"拖动条件 1",exact:true}),dialog.getByRole("group",{name:"条件 2",exact:true}));
  const preview=page.locator(".note-block-pointer-preview");
  await expect(preview).toBeVisible();
  expect(await preview.evaluate(element=>{
    if(!element.closest("dialog")?.open)return false;
    const previous=element.style.pointerEvents;element.style.pointerEvents="auto";
    const rect=element.getBoundingClientRect(),hit=element.ownerDocument.elementFromPoint(rect.x+rect.width/2,rect.y+rect.height/2);
    element.style.pointerEvents=previous;return hit===element;
  })).toBe(true);
  await page.screenshot({path:".tmp/verification/nal-shared-block-native-dialog.png"});
  await page.mouse.up();
  await expect(dialog.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("?文章 → 深度学习领域");
  await dialog.getByRole("button",{name:"撤销语句编辑",exact:true}).click();
  await expect(dialog.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("?文章 → AI领域");
});

test("floating controls leave nested text layout unchanged, match note controls and expose no drag hint",async({page})=>{
  await page.goto("/nal-editor.html");await page.getByRole("button",{name:"嵌套分组",exact:true}).click();
  const heading=page.getByRole("group",{name:"分组 1",exact:true}).locator(":scope > [data-nal-block-content]");
  const child=page.getByRole("group",{name:"条件 1.1",exact:true}).locator(":scope > [data-nal-block-content]");
  await expect(child.getByRole("button",{name:"条件 1.1 连接符",exact:true})).toHaveCSS("border-width","0px");
  const nested=page.getByRole("group",{name:"分组 1.2",exact:true}).locator(":scope > [data-nal-block-content]");
  const leaf=page.getByRole("group",{name:"条件 1.2.1",exact:true}).locator(":scope > [data-nal-block-content]");
  const before=await Promise.all([heading,child,nested,leaf].map(row=>row.boundingBox()));
  expect(before[1]!.x-before[0]!.x).toBeCloseTo(24,0);expect(before[3]!.x-before[2]!.x).toBeCloseTo(24,0);
  await child.hover();
  const toolbar=page.getByRole("toolbar",{name:"块控件：条件 1.1",exact:true});
  await expect(toolbar).toHaveCSS("opacity","1");
  await expect(page.locator(".block-floating-controls[data-active=true]")).toHaveCount(1);
  await expect(async()=>{
    const rect=await toolbar.boundingBox(), anchor=await child.boundingBox();
    expect(rect!.x+rect!.width).toBeCloseTo(anchor!.x-4,0);expect(rect!.y).toBeCloseTo(anchor!.y,0);
  }).toPass();
  const after=await Promise.all([heading,child,nested,leaf].map(row=>row.boundingBox()));
  expect(after).toEqual(before);
  const handle=page.getByRole("button",{name:"拖动条件 1.1",exact:true});
  await handle.hover();await expect(handle).not.toHaveAttribute("title");await expect(handle).not.toHaveAttribute("aria-describedby");
  // Hint owners use a 300ms delay; wait past it to verify none is created.
  await page.waitForTimeout(350);await expect(page.getByRole("tooltip")).toHaveCount(0);
  const styleOf=(element:HTMLElement)=>{
    const s=getComputedStyle(element);return {width:s.width,height:s.height,border:s.borderWidth,radius:s.borderRadius,background:s.backgroundColor,color:s.color,padding:s.padding};
  };
  const nalStyle=await handle.evaluate(styleOf);
  expect(nalStyle.border).toBe("0px");
  await page.screenshot({path:".tmp/verification/nal-floating-nested-controls.png"});
  await page.goto("/");
  await page.evaluate(async()=>{const file="/ra-workbench/apps/web/src/core/components/block-editor/NoteBlockActions.fixture.tsx";(await import(file)).mountNoteBlockActionsFixture();});
  const host=page.getByRole("region",{name:"普通笔记",exact:true});
  await host.locator('.bn-block[data-id="tail"] > .bn-block-content').hover();
  const noteHandle=page.getByRole("button",{name:"拖动块；点击打开块菜单",exact:true});
  await noteHandle.hover();
  await expect(noteHandle).not.toHaveAttribute("title");await expect(noteHandle).not.toHaveAttribute("aria-describedby");
  expect(await noteHandle.evaluate(styleOf)).toEqual(nalStyle);
  await page.screenshot({path:".tmp/verification/note-floating-shared-controls.png"});
  await host.locator('.bn-block[data-id="tail"] > .bn-block-content').hover();
  await page.getByRole("button",{name:"添加块",exact:true}).click();
  await expect(host.locator(".bn-block-content")).toHaveCount(3);
  await expect(page.locator(".bn-suggestion-menu")).toBeVisible();
});

test("moving straight up or down in the floating handle gutter uses the intended sibling boundary",async({page})=>{
  await page.goto("/nal-editor.html");await addBlock(page);await editSlot(page,"条件 2 连接符","==>（蕴含）");
  const moveInGutter=async(from:number,to:number)=>{
    const row=page.getByRole("group",{name:`条件 ${from}`,exact:true});await row.hover();
    const handle=page.getByRole("button",{name:`拖动条件 ${from}`,exact:true});
    const source=await handle.boundingBox(),target=await page.getByRole("group",{name:`条件 ${to}`,exact:true}).locator(":scope > [data-nal-block-content]").boundingBox();
    if(!source||!target)throw new Error("Missing gutter geometry");
    await page.mouse.move(source.x+source.width/2,source.y+source.height/2);await page.mouse.down();
    await page.mouse.move(source.x+source.width/2,target.y+target.height/2,{steps:12});
    await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
    await page.mouse.up();
  };
  await moveInGutter(2,1);
  await expect(page.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("[选择词项] ⇒ [选择词项]");
  await moveInGutter(1,2);
  await expect(page.getByLabel("条件 1 Narsese 语句",{exact:true})).toHaveText("(?文章 × AI领域) → 所属领域");
});

test("every add command inserts the next sibling for groups and statements at every query nesting level",async({page})=>{
  await page.goto("/nal-editor.html");await page.getByRole("button",{name:"嵌套分组",exact:true}).click();
  for(const label of ["分组 1","分组 1.2","条件 1.1","条件 1.2.1"]){
    for(const kind of ["statement","conjunction 分组","disjunction 分组"]){
      await page.getByRole("button",{name:"重置当前示例",exact:true}).click();
      const row=page.getByRole("group",{name:label,exact:true});
      const before=await row.evaluate(element=>({
        id:element.getAttribute("data-nal-block-id"),
        siblings:[...element.parentElement!.children].map(node=>node.getAttribute("data-nal-block-id")),
        subtree:element.querySelector(":scope > .nal-query-block-list")?.textContent??null,
      }));
      const at=before.siblings.indexOf(before.id);
      await addBlock(page,label,kind);
      const after=await row.evaluate(element=>({
        siblings:[...element.parentElement!.children].map(node=>node.getAttribute("data-nal-block-id")),
        subtree:element.querySelector(":scope > .nal-query-block-list")?.textContent??null,
        nextGroup:element.nextElementSibling?.classList.contains("nal-query-group"),
        nextChildren:element.nextElementSibling?.querySelector(":scope > .nal-query-block-list")?.children.length??0,
      }));
      expect(after.siblings).toHaveLength(before.siblings.length+1);
      expect(after.siblings.slice(0,at+1)).toEqual(before.siblings.slice(0,at+1));
      expect(after.siblings.slice(at+2)).toEqual(before.siblings.slice(at+1));
      expect(after.subtree).toBe(before.subtree);
      expect(after.nextGroup).toBe(kind!=="statement");expect(after.nextChildren).toBe(kind==="statement" ? 0 : 2);
      if(label==="分组 1"&&kind==="statement")await page.screenshot({path:".tmp/verification/nal-group-add-next-sibling.png"});
      await page.getByRole("button",{name:"撤销语句编辑",exact:true}).click();
      expect(await row.evaluate(element=>[...element.parentElement!.children].map(node=>node.getAttribute("data-nal-block-id")))).toEqual(before.siblings);
      await page.getByRole("button",{name:"重做语句编辑",exact:true}).click();
      expect(await row.evaluate(element=>[...element.parentElement!.children].map(node=>node.getAttribute("data-nal-block-id")))).toEqual(after.siblings);
    }
  }
});

test("production group add uses the same sibling rule and leaves existing children intact",async({page})=>{
  await page.goto("/?preview=app");await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
  await editSlot(page,"条件 1 连接符","disjunction（∨）");
  await addBlock(page,"分组 1");
  await expect(page.getByRole("group",{name:"条件 2",exact:true})).toBeVisible();
  await expect(page.getByRole("group",{name:"条件 1.3",exact:true})).toHaveCount(0);
  await editSlot(page,"条件 1.1 连接符","conjunction（∧）");
  await addBlock(page,"分组 1.1","disjunction 分组");
  await expect(page.getByRole("button",{name:"分组 1.2 connector",exact:true})).toHaveText("∨ disjunction");
  await expect(page.getByRole("group",{name:"条件 1.1.3",exact:true})).toHaveCount(0);
  await expect(page.getByRole("group",{name:"条件 1.3",exact:true})).toBeVisible();
});

for(const route of ["/nal-editor.html","/?preview=app"]){
  test(`query blocks keep only normal block spacing without internal vertical padding: ${route}`,async({page})=>{
    await page.setViewportSize({width:1146,height:730});await page.goto(route);
    if(route==="/nal-editor.html")await page.getByRole("button",{name:"嵌套分组",exact:true}).click();
    else{
      await page.getByRole("button",{name:"设置文献查询",exact:true}).click();
      await editSlot(page,"条件 1 连接符","disjunction（∨）");
      await editSlot(page,"条件 1.1 连接符","conjunction（∧）");
    }
    await addBlock(page,"分组 1");
    for(const width of [1146,420]){
      await page.setViewportSize({width,height:730});
      const layout=await page.locator(".nal-query-builder").evaluate(root=>({
        paddings:[...root.querySelectorAll(".nal-query-block,.nal-query-group-heading,.nal-inline-statement")].map(node=>{
          const style=getComputedStyle(node);return [style.paddingTop,style.paddingBottom];
        }),
        lists:[...root.querySelectorAll(".nal-query-block-list")].map(list=>{
          const rows=[...list.children].map(node=>node.getBoundingClientRect());
          return {gap:getComputedStyle(list).rowGap,distances:rows.slice(1).map((row,i)=>row.top-rows[i]!.bottom)};
        }),
        headerGaps:[...root.querySelectorAll(".nal-query-group")].map(group=>{
          const heading=group.querySelector(":scope > .nal-query-group-heading"),children=group.querySelector(":scope > .nal-query-block-list");
          return heading&&children ? children.getBoundingClientRect().top-heading.getBoundingClientRect().bottom : 0;
        }),
      }));
      expect(layout.paddings.length).toBeGreaterThan(3);
      for(const padding of layout.paddings)expect(padding).toEqual(["0px","0px"]);
      for(const list of layout.lists){expect(list.gap).toBe("8px");for(const distance of list.distances)expect(distance).toBeCloseTo(8,1);}
      for(const gap of layout.headerGaps)expect(gap).toBeCloseTo(0,1);
      if(width===1146)await page.screenshot({path:route==="/nal-editor.html" ? ".tmp/verification/nal-block-spacing.png" : ".tmp/verification/nal-block-spacing-production.png"});
    }
  });
}
