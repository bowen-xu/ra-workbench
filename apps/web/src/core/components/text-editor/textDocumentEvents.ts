import type { WorkspaceView } from "../../useWorkspaceNavigation";
import type {AgentResourceScope,AgentSkillIdentity} from "@research-assistant/core/agent";

export type TextDocumentReopen = {readonly kind:"system"; readonly id:string}
  | {readonly kind:"skill"; readonly identity:AgentSkillIdentity; readonly scope?:AgentResourceScope};

export interface TextDocumentRequest {
  /** Reopen through the authoritative gateway, never serialize save callbacks. */
  readonly reopen?: TextDocumentReopen;
  /** Stable identity supplied by the resource/file gateway, never a tab index. */
  readonly id: string;
  readonly path: string;
  readonly title?: string;
  readonly owner?: WorkspaceView;
  readonly content: string;
  readonly baseline?: string;
  readonly readOnly?: boolean;
  /** The gateway owns permission, revision checks and persistence. */
  readonly save?: (content: string, baseline: string) => Promise<void>;
}
export const OPEN_TEXT_DOCUMENT = "research-assistant:open-text-document";
/** Emitted only when the destination has accepted navigation and activated a tab. */
export const TEXT_DOCUMENT_REVEALED = "research-assistant:text-document-revealed";
export function openTextDocument(document: TextDocumentRequest): boolean {
  const event = new CustomEvent<TextDocumentRequest>(OPEN_TEXT_DOCUMENT, { detail: document, cancelable: true });
  window.dispatchEvent(event);
  return event.defaultPrevented;
}
