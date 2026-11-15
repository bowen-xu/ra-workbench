import { decodeAgentUserInput } from "@research-assistant/core/agent";
import { isAgentCanvasScenario, type AgentCanvasScenario } from "../../../core/features/agent/agentCanvasScenario";
import { nativeCanvasScenarioOptions, type NativeCanvasScenario } from "./nativeCanvasScenarios";
import { canvasFixtureError, isNativeCanvasId, isNativeCanvasSettings, type NativeCanvasChoice, type NativeCanvasSettings, type NativeCanvasStart } from "./nativeCanvasScenarioTypes";
import type { NativeAgentPreviewSample } from "../../../core/runtime/nativeAgentPreview";

export const NATIVE_CANVAS_MANIFEST_KEY="research-assistant:agent-canvas-native-fixture:v1";
export interface NativeCanvasDisposal {
  readonly fixtureId:string;readonly conversationId:string;readonly manuscriptId:string;readonly activePath:string;readonly startRequestId:string;
}
export interface NativeCanvasManifest {
  readonly schemaVersion:1;readonly fixtureId:string;readonly choice:NativeCanvasChoice;readonly scenario:AgentCanvasScenario;
  readonly settings:NativeCanvasSettings;readonly responseTexts?:readonly string[];readonly samples?:readonly NativeAgentPreviewSample[];
  /** Last confirmed live settings; settings above remains the original Create fingerprint. */
  readonly currentSettings?:NativeCanvasSettings;
  readonly stage:"creating"|"ready"|"starting"|"initialized";readonly start?:NativeCanvasStart;
  /** Durable reset locator for the original owner; never a stop or file-save receipt. */
  readonly disposal?:NativeCanvasDisposal;
  /** Original selected locator retained while a new fixture has no receipt. */
  readonly rollback?:NativeCanvasManifest;
}
function validateDisposal(row:Partial<NativeCanvasManifest>):void{
  const value=row.disposal;if(value===undefined)return;
  if(!value||typeof value!=="object"||!isNativeCanvasId(value.fixtureId)||value.fixtureId!==row.fixtureId
    ||!isNativeCanvasId(value.conversationId)||!isNativeCanvasId(value.manuscriptId)||!isNativeCanvasId(value.startRequestId)
    ||typeof value.activePath!=="string"||!value.activePath||value.activePath.includes("\0")||value.activePath.startsWith("/")||value.activePath.split("/").includes("..")
    ||(row.start!==undefined&&row.start.requestId!==value.startRequestId))throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_INVALID");
}
function disposalIdentity(value:NativeCanvasManifest):string{
  const item=value.disposal;return JSON.stringify(item?[item.fixtureId,item.conversationId,item.manuscriptId,item.activePath,item.startRequestId]:null);
}
export function isNativeCanvasChoice(value:string):value is NativeCanvasChoice {
  return isAgentCanvasScenario(value)||(value.startsWith("wire:")&&nativeCanvasScenarioOptions.some(([name])=>name===value.slice(5)));
}
export function wireCanvasScenario(choice:NativeCanvasChoice):NativeCanvasScenario|null{return choice.startsWith("wire:")?choice.slice(5) as NativeCanvasScenario:null;}
function decodeManifest(value:unknown, allowRollback:boolean):NativeCanvasManifest{
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error();
    const row=value as Partial<NativeCanvasManifest>;
    if(row.schemaVersion!==1||!isNativeCanvasId(row.fixtureId)||typeof row.choice!=="string"||!isNativeCanvasChoice(row.choice)
      ||typeof row.scenario!=="string"||!isAgentCanvasScenario(row.scenario)||!isNativeCanvasSettings(row.settings)
      ||typeof row.stage!=="string"||!["creating","ready","starting","initialized"].includes(row.stage))throw new Error();
    if(row.responseTexts!==undefined&&(!Array.isArray(row.responseTexts)||row.responseTexts.some(value=>typeof value!=="string")))throw new Error();
    if(row.samples!==undefined&&(!Array.isArray(row.samples)||row.samples.some(value=>!value||typeof value.sse!=="string")))throw new Error();
    if(row.currentSettings!==undefined&&!isNativeCanvasSettings(row.currentSettings))throw new Error();
    if(row.start){if(!isNativeCanvasId(row.start.requestId)||typeof row.start.text!=="string")throw new Error();
      if(row.start.queuedTexts!==undefined&&(!Array.isArray(row.start.queuedTexts)||row.start.queuedTexts.length>64||row.start.queuedTexts.some(value=>typeof value!=="string")))throw new Error();
      decodeAgentUserInput(row.start.userInput,row.start.text);}
    validateDisposal(row);
    if(row.rollback!==undefined) {
      if(!allowRollback)throw new Error();
      decodeManifest(row.rollback,false);
    }
    return row as NativeCanvasManifest;
}
export function readNativeCanvasManifest(storage:Storage):NativeCanvasManifest|null{
  const raw=storage.getItem(NATIVE_CANVAS_MANIFEST_KEY);if(raw===null)return null;
  try{
    return decodeManifest(JSON.parse(raw),true);
  }catch{throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_INVALID");}
}
function sameCreation(a:NativeCanvasManifest,b:NativeCanvasManifest):boolean{
  return JSON.stringify([a.fixtureId,a.choice,a.scenario,a.settings,a.responseTexts,a.samples,a.rollback])===JSON.stringify([b.fixtureId,b.choice,b.scenario,b.settings,b.responseTexts,b.samples,b.rollback]);
}
export function saveNativeCanvasManifest(storage:Storage,value:NativeCanvasManifest):void{
  validateDisposal(value);
  const previous=readNativeCanvasManifest(storage);
  if(previous&&(!sameCreation(previous,value)||(previous.start&&JSON.stringify(previous.start)!==JSON.stringify(value.start))
    ||(previous.disposal&&(disposalIdentity(previous)!==disposalIdentity(value)||previous.stage!==value.stage
      ||JSON.stringify(previous.start)!==JSON.stringify(value.start)||JSON.stringify(previous.currentSettings)!==JSON.stringify(value.currentSettings)))))throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_CONFLICT");
  storage.setItem(NATIVE_CANVAS_MANIFEST_KEY,JSON.stringify(value));
}
/** Atomically advance the current locator without disposing the previous owner. */
export function replaceNativeCanvasManifest(storage: Storage, expected: NativeCanvasManifest, value: NativeCanvasManifest): void {
  validateDisposal(value);
  const previous = readNativeCanvasManifest(storage);
  if (!previous || JSON.stringify(previous) !== JSON.stringify(expected)) throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_CONFLICT");
  storage.setItem(NATIVE_CANVAS_MANIFEST_KEY, JSON.stringify(value));
}
export function clearNativeCanvasManifest(storage:Storage,expected:NativeCanvasManifest):void{
  const previous=readNativeCanvasManifest(storage);
  if(previous&&(!sameCreation(previous,expected)||JSON.stringify(previous.start)!==JSON.stringify(expected.start)
    ||disposalIdentity(previous)!==disposalIdentity(expected)||(previous.disposal&&(previous.stage!==expected.stage
      ||JSON.stringify(previous.currentSettings)!==JSON.stringify(expected.currentSettings)))))throw canvasFixtureError("CANVAS_FIXTURE_LOCATOR_CONFLICT");
  storage.removeItem(NATIVE_CANVAS_MANIFEST_KEY);
}
