import {publicPackagePath} from "../ui/projectPaths";
import {test,expect} from "@playwright/test";
import {verifyWritingCatalogFlow} from "./writing-catalog-flow.browser";
import {verifyWritingCatalogLarge} from "./writing-catalog-large.browser";
import {verifyWritingCatalogLegacy} from "./writing-catalog-legacy.browser";
import {verifyWritingCatalogEscaped} from "./writing-catalog-escaped.browser";

const paths = {writingFile:publicPackagePath("@research-assistant/writing"),dataFile:publicPackagePath("@research-assistant/data/notes")};
test.setTimeout(120_000);
test("writing catalog keeps file/CAS/search projections atomic without listing bodies",async({page})=>{
  const result=await verifyWritingCatalogFlow(page,paths);
  expect(result).toMatchObject({stale:true,rollback:true}); console.log("writing-catalog-flow",JSON.stringify(result));
});
test("2400 writing notes use bounded persistent ordering and lazy search",async({page})=>{
  const result=await verifyWritingCatalogLarge(page,paths);
  expect(result).toMatchObject({rows:2400,cases:64,maxHeaders:22,coldWasmRequests:0}); console.log("writing-catalog-large",JSON.stringify(result));
});
test("legacy writing headers prepare in bounded batches and cannot overwrite a newer save",async({page})=>{
  const result=await verifyWritingCatalogLegacy(page,paths);
  expect(result).toMatchObject({upgradeBodies:0,first:{processedCount:8,pendingCount:4},second:{processedCount:4,pendingCount:0},noOp:{processedCount:0},v1ToV3:true,concurrentSave:{processedCount:0},scoped:{emptyScope:{processedCount:0},archived:{processedCount:8,pendingCount:3,errorCount:8},active:{processedCount:8,pendingCount:3,errorCount:8}}});
  console.log("writing-catalog-legacy",JSON.stringify(result));
});
test("valid long escaped search terms retain bounded scoped pagination",async({page})=>{
  const result=await verifyWritingCatalogEscaped(page,paths);
  expect(result).toMatchObject({pages:[20,5],scoped:true}); console.log("writing-catalog-escaped",JSON.stringify(result));
});
