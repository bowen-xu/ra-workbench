import { decodeNoteDocument, noteBlockId, type NoteBlockContentV1, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";

export const noteExternalActions = [
  ["paragraph", "段落", "块内容"], ["heading", "标题", "块内容"],
  ["bulletListItem", "无序列表", "块内容"], ["numberedListItem", "有序列表", "块内容"],
  ["checkListItem", "待办列表", "块内容"], ["quote", "引用", "块内容"], ["codeBlock", "代码块", "块内容"],
  ["divider", "分割线", "块内容"], ["formulaBlock", "块级公式", "块内容"], ["inlineFormula", "行内公式", "块内容"],
  ["image", "图片及说明", "块内容"], ["file", "附件及说明", "块内容"],
  ["childPage", "已有子页面块（旧格式）", "块内容"], ["unsupported", "未知块保全", "块内容"],
  ["inlineStyles", "文字样式、链接与提及", "块内容"], ["nestedText", "嵌套段落", "块内容"],
  ["tableCell", "单元格", "结构变化"], ["tableRowAdd", "插入行", "结构变化"],
  ["tableRowDelete", "删除行", "结构变化"], ["tableRowMove", "移动行", "结构变化"],
  ["tableColumnAdd", "插入列", "结构变化"], ["tableColumnDelete", "删除列", "结构变化"],
  ["tableColumnMove", "移动列", "结构变化"], ["blockAdd", "新增块", "结构变化"],
  ["blockDelete", "删除块", "结构变化"], ["blockMove", "移动块", "结构变化"],
  ["nestedMove", "跨父级移动", "结构变化"],
].map(([id, label, group]) => ({ id: id!, label: label!, group: group! }));
export interface ExampleAsset { readonly id: string; readonly displayName: string; readonly mimeType: string }
export interface ExampleAssets { readonly image: ExampleAsset; readonly file: ExampleAsset }
const inline = (text: string): NoteBlockContentV1 => ({kind:"inline",items:[{type:"text",text,styles:[]}]});
const create = (id: string, type: NoteBlockV1["type"], content: NoteBlockContentV1, props: NoteBlockV1["props"] = {}): NoteBlockV1 =>
  ({id:noteBlockId(id),type,content,props,children:[]});
const missing = (message: string): never => { throw new Error(`MODEL_FIXTURE_RANGE_CHANGED：${message}`); };
function mapBlock(document: NoteDocumentV1, id: string, update: (block: NoteBlockV1) => NoteBlockV1, fallback?: NoteBlockV1): NoteDocumentV1 {
  let found = false;
  const visit = (blocks: readonly NoteBlockV1[]): readonly NoteBlockV1[] => blocks.map(block => {
    if (block.id === id) { found = true; return update(block); }
    return {...block,children:visit(block.children)};
  });
  const blocks = visit(document.blocks);
  if (!found && !fallback) return missing(`目标 ${id} 已移除；可先重置或重新添加该块。`);
  return {...document,blocks:found ? blocks : [...blocks,fallback!]};
}
/** Synthetic external input only. Decode, diff, merge and persistence use the
 * real document adapter and repository, never scenario-specific algorithms. */
export function changeNoteExample(document: NoteDocumentV1, operation: string, version: number, assets?: ExampleAssets): NoteDocumentV1 {
  const stamp = `外部修改 v${version}`;
  let result = document;
  const put = (id: string, block: NoteBlockV1) => mapBlock(document,id,current=>({...block,id:current.id,props:{...current.props,...block.props},children:current.children}),block);
  if (["paragraph","heading","bulletListItem","numberedListItem","checkListItem","quote","codeBlock"].includes(operation)) {
    const type = operation as NoteBlockV1["type"];
    const id = operation === "paragraph" ? "hbs-score" : operation === "heading" ? "hbs-title" : `hbs-demo-${operation}`;
    const props = type === "heading" ? {level:version % 3 + 1} : type === "checkListItem" ? {checked:version % 2 === 0}
      : type === "codeBlock" ? {language:"python"} : {};
    result = put(id,create(id,type,inline(type === "codeBlock" ? `score = ${version}\nprint(score)` : `${stamp} · ${operation}`),props));
  } else if (operation === "divider") {
    const id="hbs-demo-divider";
    result=put(id,create(id,"divider",{kind:"none"},{backgroundColor:version % 2 ? "blue" : "default"}));
  } else if (operation === "formulaBlock") {
    result=put("hbs-formula",create("hbs-formula","formulaBlock",{kind:"formula",source:`E = ${version}mc^2`}));
  } else if (operation === "inlineFormula") {
    result=mapBlock(document,"hbs-inline",block=>({...block,content:{kind:"inline",items:[{type:"text",text:"行内公式：",styles:[]},{type:"inlineFormula",source:`x^{${version}}`}]}}));
  } else if (operation === "image" || operation === "file") {
    const asset=assets?.[operation]; if(!asset)return missing("真实附件尚未导入，未构造虚假资产身份。");
    const id=`hbs-demo-${operation}`;
    result=put(id,create(id,operation,{kind:"asset",assetId:asset.id,displayName:asset.displayName,mimeType:asset.mimeType,caption:stamp}));
  } else if (operation === "childPage") {
    const find = (blocks:readonly NoteBlockV1[]):NoteBlockV1|undefined => blocks.find(block=>block.type === "childPage") ?? blocks.flatMap(block=>{const child=find(block.children);return child?[child]:[];})[0];
    const child=find(document.blocks);
    if(!child || child.content.kind !== "reference")return missing("当前笔记没有真实子页面块；不会伪造父子关系。此项用于核对已有旧格式子页面。");
    result=mapBlock(document,child.id,block=>({...block,content:{...child.content as Extract<NoteBlockContentV1,{kind:"reference"}>,labelSnapshot:stamp}}));
  } else if (operation === "unsupported") {
    const id="hbs-demo-unsupported";
    result=put(id,create(id,"unsupported",{kind:"unsupported",originalType:"syntheticFutureBlock",rawJson:JSON.stringify({version,payload:{message:stamp}})}));
  } else if (operation === "inlineStyles") {
    const id="hbs-demo-styled";
    result=put(id,create(id,"paragraph",{kind:"inline",items:[{type:"text",text:stamp,styles:["bold","italic"],textColor:version % 2 ? "blue" : "green"},
      {type:"link",href:`https://example.org/hbs/${version}`,content:[{type:"text",text:" 合成链接",styles:["underline"]}]},
      {type:"mention",targetKind:"paper",targetId:"synthetic-paper",labelSnapshot:" 合成文献提及"}]}));
  } else if (operation === "nestedText") {
    result=mapBlock(document,"hbs-nested-a",block=>({...block,content:inline(stamp)}));
  } else if (operation.startsWith("table")) {
    result=mapBlock(document,"hbs-table",block=>{
      if(block.content.kind !== "table")return missing("表格目标已改变类型。");
      const rows=block.content.rows.map(row=>[...row]); const width=rows[0]?.length ?? 0;
      const cell=(text:string)=>({kind:"inline" as const,items:[{type:"text" as const,text,styles:[]}]});
      if(!width || rows.some(row=>row.length!==width))return missing("当前不是规则表格，未按下标猜测对应关系。");
      if(operation === "tableCell") { if(!rows[1]?.[1])return missing("目标单元格不存在，请先插入行列。"); rows[1][1]=cell(`0.${version}`); }
      else if(operation === "tableRowAdd")rows.splice(1,0,Array.from({length:width},(_,column)=>cell(`新增行 ${version}:${column}`)));
      else if(operation === "tableRowDelete") { if(rows.length<=1)return missing("至少保留一行。");rows.pop(); }
      else if(operation === "tableRowMove") { if(rows.length<3)return missing("移动数据行需要至少三行，请先插入行。");rows.push(rows.splice(1,1)[0]!); }
      else if(operation === "tableColumnAdd")rows.forEach((row,index)=>row.splice(1,0,cell(`新增列 ${version}:${index}`)));
      else if(operation === "tableColumnDelete") { if(width<=1)return missing("至少保留一列。");rows.forEach(row=>row.pop()); }
      else if(operation === "tableColumnMove") { if(width<2)return missing("移动列需要至少两列。");rows.forEach(row=>row.push(row.shift()!)); }
      else return missing("未知表格操作。");
      return {...block,content:{kind:"table",rows}};
    });
  } else if (operation === "blockAdd") {
    result={...document,blocks:[...document.blocks,create(`hbs-added-${version}`,"paragraph",inline(stamp))]};
  } else if (operation === "blockDelete") {
    const target=[...document.blocks].reverse().find(block=>block.id.startsWith("hbs-added-")) ?? document.blocks.find(block=>block.id === "hbs-next");
    if(!target)return missing("没有可删除的样例块，请先新增块。");
    result={...document,blocks:document.blocks.filter(block=>block.id!==target.id)};
  } else if (operation === "blockMove") {
    const blocks=[...document.blocks];if(blocks.length<2)return missing("移动块需要至少两个顶层块。");
    blocks.splice(1,0,blocks.pop()!);result={...document,blocks};
  } else if (operation === "nestedMove") {
    let moved:NoteBlockV1|undefined;
    result=mapBlock(document,"hbs-method",block=>{moved=block.children.at(-1);return {...block,children:block.children.slice(0,-1)};});
    if(!moved)return missing("源父块没有可移动子块。");
    result=mapBlock(result,"hbs-score",block=>({...block,children:[...block.children,moved!]}));
  } else return missing(`未知操作 ${operation}`);
  return decodeNoteDocument(result);
}
