// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import type { AgentEvidenceInput } from "@research-assistant/core/agent";
import { reconcileEvidenceDrafts, releaseRemovedEvidence, retainHistoryEvidence, releaseHistoryEvidence } from "./evidenceImageGateway";

const ports = vi.hoisted(() => {
  const values = new Map<string, string>();
  return {invoke: vi.fn().mockResolvedValue(undefined), storage: {
    get length() {return values.size;}, key: (index: number) => [...values.keys()][index] ?? null,
    getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => {values.set(key, value);},
    removeItem: (key: string) => {values.delete(key);}, clear: () => values.clear(),
  }};
});
vi.mock("../../runtime/platformCore", () => ({invoke: ports.invoke}));
vi.mock("../../runtime/appRuntime", () => ({hasPlatformCapabilities: () => true}));
vi.mock("../../runtime/appStorage", () => ({getAppStorage: () => ports.storage}));
const annotation: AgentEvidenceInput = {type: "annotation", source: "source://notes/note", title: "图片", text: "", comment: "",
  image: {assetId: "a".repeat(64), captureId: "00000000-0000-4000-8000-000000000001", mediaType: "image/png", byteSize: 10}};
afterEach(() => {ports.storage.clear(); vi.clearAllMocks();});

it("keeps a capture still owned by another persisted draft and releases only after its last draft is removed", () => {
  ports.storage.setItem("research-assistant:independent-input:other", JSON.stringify([annotation]));
  releaseRemovedEvidence([annotation], []);
  expect(ports.invoke).not.toHaveBeenCalled();
  ports.storage.removeItem("research-assistant:independent-input:other");
  releaseRemovedEvidence([annotation], []);
  expect(ports.invoke).toHaveBeenCalledWith("agent_release_evidence_image", {captureId: annotation.image!.captureId});
});
it("reconciles only app-owned draft captures, excluding unrelated storage", async () => {
  ports.storage.setItem("research-assistant.agent-drafts.v1", JSON.stringify({draft: {evidence: [annotation]}}));
  ports.storage.setItem("unrelated", JSON.stringify([annotation]));
  await reconcileEvidenceDrafts();
  expect(ports.invoke).toHaveBeenCalledWith("agent_reconcile_evidence_drafts", {captureIds: [annotation.image!.captureId]});
});
it("leaves native holds intact when persistent drafts cannot be decoded", async () => {
  ports.storage.setItem("research-assistant.agent-drafts.v1", "corrupt");
  releaseRemovedEvidence([annotation], []);
  await reconcileEvidenceDrafts();
  expect(ports.invoke).not.toHaveBeenCalled();
});
it("retains removed image captures for composer undo and releases only when the last history owner closes", () => {
  const first={}, second={};
  retainHistoryEvidence(first,[annotation]); retainHistoryEvidence(second,[annotation]);
  releaseRemovedEvidence([annotation],[]);
  expect(ports.invoke).not.toHaveBeenCalled();
  releaseHistoryEvidence(first);expect(ports.invoke).not.toHaveBeenCalled();
  releaseHistoryEvidence(second);
  expect(ports.invoke).toHaveBeenCalledExactlyOnceWith("agent_release_evidence_image",{captureId:annotation.image!.captureId});
});
