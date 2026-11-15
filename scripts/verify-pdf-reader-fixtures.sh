#!/usr/bin/env bash
set -euo pipefail

reader_repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
reader_reference_root="$reader_repo_root/docs/references/projects/zotero-reader"
reader_pdf_root="$reader_reference_root/pdfjs/pdf.js/test/pdfs"
expected_reader_ref="37a09465ca0b4f4c6ca2781c373ccf66d6c54309"
expected_pdfjs_ref="f57fc80d1c07e4cdc50a767ae0b500b5272123b4"

if [[ "$(git -C "$reader_reference_root" rev-parse HEAD)" != "$expected_reader_ref" ]]; then
  printf 'Zotero Reader fixture checkout is not at %s\n' "$expected_reader_ref" >&2
  exit 1
fi
if [[ "$(git -C "$reader_reference_root/pdfjs/pdf.js" rev-parse HEAD)" != "$expected_pdfjs_ref" ]]; then
  printf 'Zotero PDF.js fixture checkout is not at %s\n' "$expected_pdfjs_ref" >&2
  exit 1
fi

verify_fixture() {
  local expected_hash="$1"
  local fixture_name="$2"
  printf '%s  %s\n' "$expected_hash" "$reader_pdf_root/$fixture_name" | shasum -a 256 -c - >/dev/null
}

verify_fixture 753441c619c529c8d0fc3e7e470f07c9547838fc7229bea8c1dcf854acee03b4 freeculture.pdf
verify_fixture 3662ff519e485810520552bf301d8c3b2b917fd2f83303f4965d7abed367e113 tracemonkey.pdf
verify_fixture 5fa399fa5e639c7d5ca03ab1f50047c4f7f1c218043ffda3eb76e99cc36437f1 sizes.pdf
verify_fixture 5087c2afe763d145537d652bb3bc6c321ace3a267b2679ae2c178add3ba7f1d9 rotation.pdf
verify_fixture 514511143db12309893fb69cdf98c76f2361c20da394f79fdff2c119ed7a4393 vertical.pdf
verify_fixture 1cad1de912ba29f89a6d8b08bc5b0f84382874ffedab2c8f9e05ef608c265bb1 ArabicCIDTrueType.pdf
verify_fixture 3a6948ca33558a0f1c9dabf70902e14e263e814b6a12da44ba8ec9439c0bafdd copy_paste_ligatures.pdf
verify_fixture 556da20f31754bcfabd6821b39e4d97ea569fd91363a19ef9b16cdec0f95a68e glyph_accent.pdf
verify_fixture 5469cf9139b28cec6fece1ac7697c98f1ea529999332d8b59535c0439f1140d1 labelled_pages.pdf
verify_fixture 1283aa2173bc57def82323ea2601a4e08ada5beb2466873aec8aa04ddd74c013 nested_outline.pdf
verify_fixture 3c2815853e6fe5c34feb76bb14402cbc46bbd484ecd57b672fc03577d0458ee8 issue15893_reduced.pdf
verify_fixture 659d6b19912f63db988b0b26b9bde0e6d8100667ef162051a4a84ea8e5b90272 cmykjpeg.pdf
verify_fixture 50e91d373aa313537972519c65b050f8d4d5bd996859fa20756432c2029c3ff6 bug1020226.pdf

printf 'Verified 13 PDF reader fixtures at the pinned Zotero PDF.js ref.\n'
