// @vitest-environment happy-dom
import { afterEach, expect, it } from "vitest";
import { blockSideMenuReviewAnchor } from "./blockSideMenuAnchor";

afterEach(() => { document.body.replaceChildren(); });

function rect(top: number): DOMRect { return new DOMRect(100, top, 400, 30); }
async function position(block: HTMLElement, y = 200) {
  const reference = { contextElement: block, getBoundingClientRect: () => rect(200) };
  const input = { y, elements: { reference } } as unknown as Parameters<typeof blockSideMenuReviewAnchor.fn>[0];
  return blockSideMenuReviewAnchor.fn(input);
}

it("anchors only the explicitly linked first candidate root to its displayed old/new start", async () => {
  document.body.innerHTML = '<div class="bn-editor"><div data-block-review-anchor-for="first"></div>'
    + '<div class="bn-block-outer" data-id="first" data-block-review-anchor="first"><div class="bn-block"></div></div>'
    + '<div class="bn-block-outer" data-id="second"><div class="bn-block"></div></div></div>';
  const old = document.querySelector<HTMLElement>("[data-block-review-anchor-for]")!;
  old.getBoundingClientRect = () => rect(120);
  expect(await position(document.querySelectorAll<HTMLElement>(".bn-block")[0]!)).toEqual({ y: 120 });
  expect(await position(document.querySelectorAll<HTMLElement>(".bn-block")[1]!)).toEqual({});
});

it("preserves nested and ordinary anchors and never finds a snapshot in another editor", async () => {
  document.body.innerHTML = '<div class="bn-editor"><div data-block-review-anchor-for="parent"></div></div>'
    + '<div class="bn-editor"><div class="bn-block-outer" data-id="parent" data-block-review-anchor="parent">'
    + '<div class="bn-block"></div><div class="bn-block-outer" data-id="child"><div class="bn-block child"></div></div></div></div>';
  expect(await position(document.querySelector<HTMLElement>(".child")!)).toEqual({});
  expect(await position(document.querySelectorAll<HTMLElement>(".bn-block")[0]!)).toEqual({});
});
