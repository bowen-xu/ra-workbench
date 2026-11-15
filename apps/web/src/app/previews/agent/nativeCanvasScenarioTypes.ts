import type { AgentUserInput } from "@research-assistant/core/agent";
import type { NativeAgentPreviewSample } from "../../../core/runtime/nativeAgentPreview";
import { isAgentCanvasScenario, type AgentCanvasScenario } from "../../../core/features/agent/agentCanvasScenario";
import type { CanvasTextSettings } from "./AgentCanvasTextSettings";
import type { CanvasQueueFailure, CanvasSteerFailure } from "../../../core/features/agent/agentCanvasFaultPorts";
import type { NativeCanvasScenario } from "./nativeCanvasScenarios";

export const NATIVE_CANVAS_CAPABILITY = "canvas-preview-parity";
export const NATIVE_CANVAS_DELETION_FAULTS_CAPABILITY = "catalog-deletion-faults";
export type NativeCanvasChoice = AgentCanvasScenario | `wire:${NativeCanvasScenario}`;
export interface NativeCanvasSettings {
  readonly durationMs: number; readonly charactersPerChunk: number; readonly intervalMs: number; readonly compactionDurationMs: number;
}
export interface NativeCanvasFixture {
  readonly schemaVersion: 1; readonly generation: string; readonly fixtureId: string; readonly scenario: AgentCanvasScenario;
  readonly conversationId: string; readonly manuscriptId: string; readonly activePath: string; readonly created: boolean; readonly startRequestId: string;
}
export interface NativeCanvasStart { readonly requestId: string; readonly text: string; readonly userInput?: readonly AgentUserInput[]; readonly queuedTexts?:readonly string[] }
export type NativeCanvasControl =
  | ({ readonly type: "start" } & NativeCanvasStart)
  | { readonly type: "rawSamples"; readonly samples: readonly NativeAgentPreviewSample[]; readonly compactionOnly?: boolean }
  | { readonly type: "remoteCompaction"; readonly v2: boolean }
  | { readonly type: "releaseSubmission" | "releaseMetadata" | "dispose" }
  | { readonly type: "queueFault" | "startFault" | "prepareFault" | "deletionFault"; readonly failure: CanvasQueueFailure | null }
  | { readonly type: "steerFault"; readonly failure: CanvasSteerFailure }
  | { readonly type: "disconnect"; readonly mode: "recover" | "exhaust" };
export function nativeCanvasSettings(durationMinutes: number, settings: CanvasTextSettings): NativeCanvasSettings {
  return { durationMs: Math.round(durationMinutes * 60000), charactersPerChunk: settings.pacing.charactersPerChunk,
    intervalMs: Math.round(settings.pacing.intervalMs), compactionDurationMs: Math.round(settings.compactionDurationMs) };
}
export function canvasFixtureError(code: string): Error & { readonly code: string } { return Object.assign(new Error(code), { code }); }
export const isNativeCanvasId = (value: unknown): value is string => typeof value === "string" && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(value) && !/^0{8}-0{4}-0{4}-0{4}-0{12}$/u.test(value);
export function decodeNativeCanvasFixture(value: unknown): NativeCanvasFixture {
  if (!value || typeof value !== "object") throw canvasFixtureError("CANVAS_FIXTURE_INVALID_RESPONSE");
  const row = value as Record<string, unknown>;
  if (row.schemaVersion !== 1 || !isNativeCanvasId(row.generation) || !isNativeCanvasId(row.fixtureId) || !isNativeCanvasId(row.conversationId) || !isNativeCanvasId(row.manuscriptId) || !isNativeCanvasId(row.startRequestId)
    || typeof row.scenario !== "string" || !isAgentCanvasScenario(row.scenario) || typeof row.created !== "boolean"
    || typeof row.activePath !== "string" || !row.activePath || row.activePath.includes("\0") || row.activePath.startsWith("/") || row.activePath.split("/").includes(".."))
    throw canvasFixtureError("CANVAS_FIXTURE_INVALID_RESPONSE");
  return row as unknown as NativeCanvasFixture;
}
export function isNativeCanvasSettings(value: unknown): value is NativeCanvasSettings {
  if (!value || typeof value !== "object") return false;
  const row = value as Record<keyof NativeCanvasSettings, unknown>;
  return [row.durationMs, row.charactersPerChunk, row.intervalMs, row.compactionDurationMs].every(value => typeof value === "number" && Number.isSafeInteger(value))
    && Number(row.durationMs) >= 0 && Number(row.charactersPerChunk) >= 1 && Number(row.intervalMs) >= 10 && Number(row.compactionDurationMs) >= 0;
}
