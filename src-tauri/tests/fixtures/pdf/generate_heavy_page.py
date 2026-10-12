#!/usr/bin/env python3
"""
Deterministic generator for heavy-page.pdf (worker timeout / page quarantine fixture).

Four pages, all small on disk. Page indexes 0 and 3 are light. Two pages are
expensive for PDFium in different ways:

- index 1: a chain of nested Form XObjects, each level drawing the one below
  FANOUT times (FANOUT ** DEPTH tiny paths). PDFium expands it while parsing
  the page, so render, thumbnail and text layout all exceed their timeouts
  and the worker's memory grows quickly. Used for manual baseline runs.
- index 2: a function-based shading with a long PostScript calculator
  function evaluated per device pixel. Only rasterizing is slow and memory
  stays small, so automated tests use this page.

Uses only the Python 3 standard library. `--verify-determinism` regenerates in
memory and compares with the committed file and its manifest entry.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parents[3]
MANIFEST_PATH = REPO_ROOT / "src-tauri" / "tests" / "fixtures" / "corpus-manifest.json"
OUTPUT_NAME = "heavy-page.pdf"
FIXTURE_ID = "core-heavy-page"

FANOUT = 10
DEPTH = 7
SHADING_ROUNDS = 7000


def stream_object(obj_id: int, dict_entries: str, data: bytes) -> bytes:
    header = f"{obj_id} 0 obj\n<< /Length {len(data)} {dict_entries} >>\nstream\n"
    return header.encode("latin1") + data + b"\nendstream\nendobj\n"


def plain_object(obj_id: int, body: str) -> bytes:
    return f"{obj_id} 0 obj\n{body}\nendobj\n".encode("latin1")


def build_pdf(objects: list[bytes]) -> bytes:
    buf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for obj in objects:
        offsets.append(len(buf))
        buf.extend(obj)
    xref = len(buf)
    buf.extend(f"xref\n0 {len(objects) + 1}\n".encode("latin1"))
    buf.extend(b"0000000000 65535 f \n")
    for offset in offsets:
        buf.extend(f"{offset:010d} 00000 n \n".encode("latin1"))
    buf.extend(
        f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(
            "latin1"
        )
    )
    return bytes(buf)


def text_content(label: str) -> bytes:
    return f"BT /F1 24 Tf 72 720 Td ({label}) Tj ET\n".encode("latin1")


def generate() -> bytes:
    # 1 catalog, 2 pages, 3 font, 4-7 page objects, 8-11 page contents,
    # 12 shading, 13 shading function, 14.. form XObjects X0 (leaf) .. X{DEPTH}.
    font_id = 3
    shading_id = 12
    function_id = 13
    first_form_id = 14
    objects: dict[int, bytes] = {}
    objects[1] = plain_object(1, "<< /Type /Catalog /Pages 2 0 R >>")
    objects[2] = plain_object(
        2, "<< /Type /Pages /Kids [4 0 R 5 0 R 6 0 R 7 0 R] /Count 4 >>"
    )
    objects[font_id] = plain_object(
        font_id, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
    )

    # Page index 1: nested forms. PDFium expands every Do while parsing the
    # page, so render, thumbnail and text layout are all slow, and the worker
    # allocates memory proportional to FANOUT ** DEPTH.
    leaf = b"0 0 m 4 0 l 4 4 l 0 4 l h f\n"
    objects[first_form_id] = stream_object(
        first_form_id, "/Type /XObject /Subtype /Form /BBox [0 0 4 4]", leaf
    )
    width = height = 4
    for level in range(1, DEPTH + 1):
        obj_id = first_form_id + level
        child = f"X{level - 1}"
        ops = []
        for index in range(FANOUT):
            # Alternate the axis so the drawing stays roughly square.
            dx, dy = (width * index, 0) if level % 2 else (0, height * index)
            ops.append(f"q 1 0 0 1 {dx} {dy} cm /{child} Do Q")
        if level % 2:
            width *= FANOUT
        else:
            height *= FANOUT
        objects[obj_id] = stream_object(
            obj_id,
            f"/Type /XObject /Subtype /Form /BBox [0 0 {width} {height}] "
            f"/Resources << /XObject << /{child} {obj_id - 1} 0 R >> >>",
            ("\n".join(ops) + "\n").encode("latin1"),
        )
    top_form_id = first_form_id + DEPTH
    scale = min(540 / width, 640 / height)

    # Page index 2: a function-based shading whose PostScript calculator
    # function runs SHADING_ROUNDS * 3 operators for every device pixel. Only
    # rasterizing is slow; parsing and text layout stay cheap and small.
    body = "{ " + "dup sin pop " * SHADING_ROUNDS + "pop pop 0.2 0.4 0.6 }"
    objects[function_id] = stream_object(
        function_id,
        "/FunctionType 4 /Domain [0 1 0 1] /Range [0 1 0 1 0 1]",
        body.encode("latin1"),
    )
    objects[shading_id] = plain_object(
        shading_id,
        "<< /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 1 0 1] "
        f"/Matrix [612 0 0 792 0 0] /Function {function_id} 0 R >>",
    )

    page_contents = {
        8: text_content("LIGHT-PAGE-ONE"),
        9: text_content("FORM-HEAVY-PAGE-TWO")
        + f"q 0.2 0.2 0.6 rg {scale:.10f} 0 0 {scale:.10f} 36 36 cm /XH Do Q\n".encode(
            "latin1"
        ),
        10: b"q /Sh0 sh Q\n" + text_content("SHADING-HEAVY-PAGE-THREE"),
        11: text_content("LIGHT-PAGE-FOUR"),
    }
    for obj_id, data in page_contents.items():
        objects[obj_id] = stream_object(obj_id, "", data)
    extra_resources = {
        5: f"/XObject << /XH {top_form_id} 0 R >> ",
        6: f"/Shading << /Sh0 {shading_id} 0 R >> ",
    }
    for page_id, content_id in ((4, 8), (5, 9), (6, 10), (7, 11)):
        objects[page_id] = plain_object(
            page_id,
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] "
            f"/Resources << /Font << /F1 {font_id} 0 R >> {extra_resources.get(page_id, '')}>> "
            f"/Contents {content_id} 0 R >>",
        )

    ordered = [objects[obj_id] for obj_id in sorted(objects)]
    expected_ids = list(range(1, len(ordered) + 1))
    if sorted(objects) != expected_ids:
        raise SystemExit(f"object ids are not contiguous: {sorted(objects)}")
    return build_pdf(ordered)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verify-determinism", action="store_true")
    parser.add_argument("--output", type=Path, default=ROOT / OUTPUT_NAME)
    args = parser.parse_args()

    data = generate()
    digest = hashlib.sha256(data).hexdigest()
    if args.verify_determinism:
        committed = (ROOT / OUTPUT_NAME).read_bytes()
        manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
        entry = next(f for f in manifest["fixtures"] if f["id"] == FIXTURE_ID)
        ok = committed == data and entry["sha256"] == digest and entry["byteLength"] == len(data)
        print(f"{OUTPUT_NAME}: {'deterministic' if ok else 'MISMATCH'} sha256={digest}")
        return 0 if ok else 1
    args.output.write_bytes(data)
    print(f"{args.output}: {len(data)} bytes sha256={digest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
