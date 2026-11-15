import { openSourceLink } from "../../agent/sourceNavigation";

export function isNoteSourceHref(href: string): boolean { return /^source:/iu.test(href); }
export function navigateNoteSource(href: string): void {
  openSourceLink(href);
}
