import {expect,test} from "@playwright/test";

test("small inline formulas remain on their text line after layout and viewport changes",async({page})=>{
 await page.goto("/?preview=app&demo=text-patch");
 const open=page.getByRole('button',{name:'打开对话',exact:true});
 await expect(open.or(page.getByRole('button',{name:'隐藏对话',exact:true}))).toBeVisible();
 if(await open.isVisible())await open.click();
 await page.getByRole('button',{name:'对话列表',exact:true}).click();
 await page.getByRole('button',{name:'修改研究笔记',exact:true}).click();
 await page.getByRole('button',{name:'隐藏对话列表',exact:true}).click();
 const formula=page.locator('.writing-agent-panel .agent-math-viewport').filter({has:page.locator('annotation',{hasText:'C_i'})});
 await expect(formula).toHaveCount(1);
 for(const width of [1280,933,1528,1280]){
  await page.setViewportSize({width,height:850});
  await formula.scrollIntoViewIfNeeded();
  await expect(formula).toHaveAttribute('data-overflow','false');
  await expect(formula).toHaveCSS('display','inline-block');
  await expect(formula).not.toHaveAttribute('role','region');
  // Observe multiple paint/layout passes: a one-frame snapshot misses the
  // former ResizeObserver feedback loop between inline and block layouts.
  const stable=await formula.evaluate(async element=>{
   const samples=[];
   for(let i=0;i<8;i++){
    await new Promise(requestAnimationFrame);
    samples.push(element.getAttribute('data-overflow'));
   }
   return samples.every(value=>value==='false');
  });
  expect(stable).toBe(true);
 }
});
