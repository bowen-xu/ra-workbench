// SPDX-License-Identifier: AGPL-3.0-or-later
// Original, code-drawn document artwork replaces Zotero's trademarked app marks.
import fs from "node:fs/promises";
import path from "node:path";
import { deflateSync } from "node:zlib";

function crc(buffer) {
  let value = 0xffffffff;
  for (const byte of buffer) {
    value ^= byte;
    for (let bit = 0; bit < 8; bit++) value = (value >>> 1) ^ (value & 1 ? 0xedb88320 : 0);
  }
  return (value ^ 0xffffffff) >>> 0;
}
function chunk(type, data) {
  const tag = Buffer.from(type);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(data.length);
  const checksum = Buffer.alloc(4);
  checksum.writeUInt32BE(crc(Buffer.concat([tag, data])));
  return Buffer.concat([length, tag, data, checksum]);
}
function png(width, height, pixel) {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(width); header.writeUInt32BE(height, 4); header[8] = 8; header[9] = 6;
  const rows = Buffer.alloc((width * 4 + 1) * height);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const color = pixel(x, y);
    Buffer.from(color).copy(rows, y * (width * 4 + 1) + 1 + x * 4);
  }
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header), chunk("IDAT", deflateSync(rows)), chunk("IEND", Buffer.alloc(0))]);
}
export async function makeIcons(output, schema) {
  const resource = await fs.readFile(path.join(output, "utilities/resource/zoteroTypeSchemaData.js"), "utf8");
  const names = new Set([...resource.matchAll(/[\w@-]+\.png/g)].map(match => match[0]));
  for (const { itemType } of schema.itemTypes) names.add(`treeitem-${itemType}.png`);
  for (const name of ["webpage-gray", "attachment-pdf", "attachment-web-link", "attachment-snapshot", "multiple"]) {
    for (const suffix of ["", "@2x", "@48px"]) names.add(`treeitem-${name}${suffix}.png`);
  }
  for (const name of ["pdf.png", "cross.png", "tick.png", "treesource-library.png", "treesource-collection.png", "spinner-16px.png", "zotero-z-16px.png", "zotero-z-32px.png", "zotero-z-16px-offline.png"]) names.add(name);
  const green = [42, 94, 64, 255], transparent = [0, 0, 0, 0], white = [246, 250, 246, 255];
  const document = size => png(size, size, (x, y) => {
    const a = x / size, b = y / size;
    if (a < .18 || a > .82 || b < .08 || b > .92) return transparent;
    return a > .27 && a < .72 && [ .35, .5, .65 ].some(line => Math.abs(b - line) < .026) ? white : green;
  });
  await fs.mkdir(path.join(output, "images"), { recursive: true });
  for (const name of names) await fs.writeFile(path.join(output, "images", name), document(name.includes("48px") ? 48 : name.includes("2x") || name.includes("32px") ? 32 : 16));
  await fs.writeFile(path.join(output, "images/progress_arcs.png"), png(21 * 16, 16, (x, y) => {
    const angle = (Math.atan2(y - 7.5, (x % 16) - 7.5) + Math.PI * 2.5) % (Math.PI * 2);
    const radius = Math.hypot((x % 16) - 7.5, y - 7.5);
    return radius > 5 && radius < 7 && angle <= Math.floor(x / 16) / 20 * Math.PI * 2 ? green : transparent;
  }));
  for (const size of [16, 32, 64, 128]) await fs.writeFile(path.join(output, `Icon-${size}.png`), document(size));
}
