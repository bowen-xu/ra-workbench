/** Project-owned PDF: an internal link and a differently sized, rotated second page. */
export function navigationPdf(): Buffer {
  const stream = (text: string) => `<< /Length ${text.length} >>\nstream\n${text}\nendstream`;
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Count 2 /Kids [3 0 R 5 0 R] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 7 0 R >> >> /Contents 4 0 R /Annots [8 0 R] >>",
    stream("BT /F1 16 Tf 30 350 Td (Jump to rotated page) Tj ET"),
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 300] /Rotate 90 /Resources << /Font << /F1 7 0 R >> >> /Contents 6 0 R >>",
    stream("BT /F1 16 Tf 30 250 Td (Rotated shared PDF fixture) Tj ET"),
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    "<< /Type /Annot /Subtype /Link /Rect [25 340 250 375] /Border [0 0 0] /Dest [5 0 R /Fit] >>",
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = objects.map((object, i) => { const offset = pdf.length; pdf += `${i + 1} 0 obj\n${object}\nendobj\n`; return offset; });
  const xref = pdf.length;
  pdf += `xref\n0 9\n0000000000 65535 f \n${offsets.map((o) => `${String(o).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf);
}
