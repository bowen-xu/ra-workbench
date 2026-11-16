import {buildWorkbenchWeb} from "./build-workbench-composition.mjs";
const receipt = await buildWorkbenchWeb();
console.log(JSON.stringify({status: receipt.status, output: receipt.output, cacheHit: receipt.cacheHit}));
