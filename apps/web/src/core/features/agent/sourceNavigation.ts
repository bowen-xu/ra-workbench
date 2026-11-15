import { parseSourceLink, type SourceLink } from "@research-assistant/core/source";
import type { AgentEvidenceInput } from "@research-assistant/core/agent";
import {notify} from "../../services/notifications";

import type { SourceNavigationLocation } from "../../contracts/sourceNavigation";
export type { SourceNavigationLocation } from "../../contracts/sourceNavigation";
export interface SourceNavigationOptions { readonly presentation?: "document"; readonly locate?: boolean; readonly annotation?: AgentEvidenceInput }

export interface SourceNavigationRequest {
  readonly link: SourceLink;
  readonly options?: SourceNavigationOptions;
  readonly resolve: (location?: SourceNavigationLocation) => void;
  readonly reject: (reason: unknown) => void;
}
export const SOURCE_NAVIGATION_EVENT = "research-assistant:navigate-source";
/** Inject the same application navigation port into every note host. */
export function openSourceLink(href: string): void {
  void navigateSource(href).catch(reason => notify.error(reason instanceof Error ? reason.message : "无法打开来源"));
}
export function navigateSource(href: string, options?: SourceNavigationOptions): Promise<SourceNavigationLocation | undefined> {
  return new Promise((resolve, reject) => {
    const link = parseSourceLink(href);
    const deadline = options?.locate ? window.setTimeout(() => reject(new Error("来源仍未完成定位，请加载后重试。")), 30_000) : undefined;
    const finish = (location?: SourceNavigationLocation) => {window.clearTimeout(deadline); resolve(location);};
    const fail = (reason: unknown) => {window.clearTimeout(deadline); reject(reason);};
    const request = new CustomEvent<SourceNavigationRequest>(SOURCE_NAVIGATION_EVENT, {
      detail: { link, resolve: finish, reject: fail, ...(options ? {options} : {}) }, cancelable: true,
    });
    if (window.dispatchEvent(request)) fail(new Error("当前界面未接入来源导航"));
  });
}
