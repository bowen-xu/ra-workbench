import { expect,type Page } from "@playwright/test";

export async function editSlot(page:Page,label:string,option:string,variable?:string){
  if(label.endsWith("连接符") || / connector(?: \d+)?$/u.test(label)){
    await page.getByRole("button",{name:label,exact:true}).click();
    await page.getByRole("menu",{name:label,exact:true}).getByRole("menuitemradio",{name:option,exact:true}).click();
    await expect(page.getByRole("menu",{name:label,exact:true})).toHaveCount(0);
    return;
  }
  await page.getByRole("button",{name:label,exact:true}).click();
  const menu=page.getByRole("menu",{name:label,exact:true});
  if(option==="statement（语句）")await menu.getByRole("menuitemradio",{name:option,exact:true}).click();
  else{
    const kind=["product（×）","conjunction（∧）","disjunction（∨）"].includes(option) ? "compound" : "atomic";
    await menu.getByRole("menuitem",{name:`${label} ${kind}`,exact:true}).click();
    let choices=page.getByRole("menu",{name:`${label} ${kind}`,exact:true});
    if(kind==="atomic" && option!=="变量" && option!=="选中文献"){
      await choices.getByRole("menuitem",{name:`${label} concept`,exact:true}).click();
      choices=page.getByRole("menu",{name:`${label} concept`,exact:true});
    }
    await choices.getByRole("menuitemradio",{name:option,exact:true}).click();
  }
  await expect(menu).toHaveCount(0);
  if(variable!==undefined){
    await page.getByRole("button",{name:label,exact:true}).click();
    const input=page.getByRole("textbox",{name:`${label}的变量名`,exact:true});
    await input.fill(variable);await input.press("Enter");
    await expect(menu).toHaveCount(0);
  }
}
export async function groupConnector(page:Page,label:string,value:"conjunction"|"disjunction"){
  await editSlot(page,label,`${value==="conjunction" ? "∧" : "∨"} ${value}`);
}
export async function copula(page:Page,number:number|string,value="-->（继承）"){
  await editSlot(page,`条件 ${number} 连接符`,value);
}
/** Construct the same relation AST through the recursive production slots. */
export async function chooseInline(page:Page,number:number,kind:"关系"|"对象",label:string){
  const left=`条件 ${number} 左槽`,right=`条件 ${number} 右槽`;
  const compound=page.getByRole("button",{name:`${left} connector 1`,exact:true});
  if(await page.getByRole("button",{name:left,exact:true}).count()===0 && await compound.count()===0)await copula(page,number);
  if(kind==="关系"){
    if(label==="继承"){
      if(await compound.count()){
        await compound.click();await page.getByRole("menuitem",{name:"清空",exact:true}).click();
      }
      await editSlot(page,left,"变量","文章");
    }else{
      if(await compound.count()===0)await editSlot(page,left,"product（×）");
      await editSlot(page,`${left} · 第 1 项`,"变量","文章");
      await editSlot(page,right,label);
    }
    return;
  }
  if(await compound.count()===0 && (await page.getByRole("button",{name:right,exact:true}).innerText())==="[选择词项]" && (await page.getByRole("button",{name:left,exact:true}).innerText())==="[选择词项]")await chooseInline(page,number,"关系","所属领域");
  const product=await compound.count()>0;
  const destination=product ? `${left} · 第 2 项` : right;
  const isPaper=await page.evaluate(async target=>{
    const path="/plugins/ra-literature-library/src/services/nalGateway.ts";
    const state:import("../../packages/core/src/nal").NalState=await (await import(path)).getNalGateway().read();
    return !state.concepts.some(concept=>concept.label===target);
  },label);
  if(!isPaper){await editSlot(page,destination,label);return;}
  await page.getByRole("button",{name:destination,exact:true}).click();
  await page.getByRole("menuitem",{name:`${destination} atomic`,exact:true}).click();
  await page.getByRole("menuitem",{name:`${destination} paper`,exact:true}).click();
  await page.getByRole("menuitemradio",{name:label,exact:true}).click();
}

export async function addBlock(page:Page,label="条件 1",kind="statement"){
  await page.getByRole("group",{name:label,exact:true}).hover();
  await page.getByRole("button",{name:"添加 block："+label,exact:true}).click();
  await page.getByRole("menuitem",{name:"添加 "+kind,exact:true}).click();
}

/** Tests prepare old textual fixtures through the same visible production slots. */
export async function fillStatements(page:Page,text:string){
  const vocabulary: import("../../packages/core/src/nal").NalVocabulary[]=await page.evaluate(async()=>{
    const networkPath="/plugins/ra-literature-library/src/services/nalGateway.ts",libraryPath="/plugins/ra-literature-library/src/services/literatureGateway.ts";
    const state:import("../../packages/core/src/nal").NalState=await (await import(networkPath)).getNalGateway().read();
    const papers:readonly {id:string;title:string}[]=await (await import(libraryPath)).getLiteratureGateway().list();
    return [...state.concepts.map(c=>({atom:{kind:"concept" as const,id:c.id},label:c.label})),...papers.map(p=>({atom:{kind:"paper" as const,id:p.id},label:p.title}))];
  });
  const {parseNal}=await import("../../../ra-workbench-sdk/packages/core/src/nal/index");
  const clauses=text.split(/\r?\n/u).filter(line=>line.trim()).map(line=>parseNal(line,vocabulary));
  const fill=async(label:string,pattern:import("../../packages/core/src/nal").NalPattern):Promise<void>=>{
    if(pattern.kind==="product"){
      await editSlot(page,label,"product（×）");
      for(let i=2;i<pattern.terms.length;i++){
        await page.getByRole("button",{name:`${label} 右括号`,exact:true}).click();
        await page.getByRole("menuitem",{name:"增加槽位",exact:true}).click();
      }
      for(const [index,term] of pattern.terms.entries())await fill(`${label} · 第 ${index+1} 项`,term);
      return;
    }
    if(pattern.kind!=="paper"&&pattern.kind!=="concept")throw new Error("Source fixture must be a ground term");
    const entry=vocabulary.find(v=>v.atom.kind===pattern.kind&&v.atom.id===pattern.id);
    if(!entry)throw new Error("Missing fixture vocabulary");
    if(pattern.kind==="concept"){await editSlot(page,label,entry.label);return;}
    await page.getByRole("button",{name:label,exact:true}).click();
    await page.getByRole("menuitem",{name:`${label} atomic`,exact:true}).click();
    await page.getByRole("menuitem",{name:`${label} paper`,exact:true}).click();
    const menu=page.getByRole("menu",{name:`${label} paper`,exact:true});
    await menu.getByRole("searchbox",{name:"搜索文献",exact:true}).fill(entry.label);
    await menu.getByRole("menuitemradio",{name:entry.label,exact:true}).click();
  };
  for(const [index,clause] of clauses.entries()){
    if(index)await addBlock(page,`条件 ${index}`);
    await copula(page,index+1);
    await fill(`条件 ${index+1} 左槽`,clause.subject);
    await fill(`条件 ${index+1} 右槽`,clause.predicate);
  }
}
