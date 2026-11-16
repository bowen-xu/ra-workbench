import { expect, type Locator } from "@playwright/test";

/** Read visible source lines without CodeMirror's empty-document placeholder. */
export async function equationSource(input: Locator): Promise<string> {
  return input.locator(".cm-line").evaluateAll(lines => lines.map(line => {
    const copy = line.cloneNode(true) as HTMLElement;
    copy.querySelectorAll(".cm-placeholder").forEach(element => element.remove());
    return copy.textContent ?? "";
  }).join("\n"));
}
export async function expectEquationSource(input: Locator, expected: string | RegExp): Promise<void> {
  if (typeof expected === "string") await expect.poll(() => equationSource(input)).toBe(expected);
  else await expect.poll(() => equationSource(input)).toMatch(expected);
}
