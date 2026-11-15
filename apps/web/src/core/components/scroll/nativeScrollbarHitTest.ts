/** Hit only the browser's measured scrollbar gutter, never a guessed edge band
 * or the entire scrollable region. CSS keeps that gutter present at opacity 0.
 * Native thumbs, track clicks, scrolling, RTL and pointer capture stay native. */
export function isOverNativeScrollbar(node: HTMLElement, clientX: number, clientY: number): boolean {
  if (!node.offsetWidth || !node.offsetHeight) return false;
  const rect = node.getBoundingClientRect();
  if (!rect.width || !rect.height) return false;
  const x = (clientX - rect.left) * node.offsetWidth / rect.width;
  const y = (clientY - rect.top) * node.offsetHeight / rect.height;
  if (x < 0 || y < 0 || x >= node.offsetWidth || y >= node.offsetHeight) return false;
  const style = node.ownerDocument.defaultView?.getComputedStyle(node);
  if (!style) return false;
  const borderLeft = parseFloat(style.borderLeftWidth) || 0;
  const borderRight = parseFloat(style.borderRightWidth) || 0;
  const borderTop = parseFloat(style.borderTopWidth) || 0;
  const borderBottom = parseFloat(style.borderBottomWidth) || 0;
  const vertical = node.offsetWidth - node.clientWidth - borderLeft - borderRight;
  const horizontal = node.offsetHeight - node.clientHeight - borderTop - borderBottom;
  const scrollable = (overflow: string): boolean => overflow === "auto" || overflow === "scroll";
  // clientLeft includes a left-side native scrollbar (e.g. RTL); border widths
  // alone cannot locate it. Ratios above account for ancestor scale transforms.
  const leftSide = node.clientLeft - borderLeft > vertical / 2;
  if (vertical > 0 && scrollable(style.overflowY) && node.scrollHeight > node.clientHeight
    && y >= borderTop && y < node.offsetHeight - borderBottom
    && (leftSide ? x >= borderLeft && x < node.clientLeft : x >= node.clientLeft + node.clientWidth && x < node.offsetWidth - borderRight)) return true;
  return horizontal > 0 && scrollable(style.overflowX) && node.scrollWidth > node.clientWidth
    && x >= borderLeft && x < node.offsetWidth - borderRight
    && y >= node.clientTop + node.clientHeight && y < node.offsetHeight - borderBottom;
}
