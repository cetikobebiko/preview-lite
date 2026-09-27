#!/usr/bin/env python3
"""
Deterministic PDF Generator & Verification Script for Antigravity PDF Reader.
Generates:
1. release-smoke.pdf (Canonical release-smoke corpus target with Turkish Unicode Type 3 font)
2. large-page-dimensions.pdf (7200x7200 pt large dimension minimal fixture)

Uses purely Python 3 standard library (no pip packages, no external network).
Provides --verify-determinism mode for non-destructive CI/test validation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO_ROOT = ROOT.parents[3]
MANIFEST_PATH = REPO_ROOT / "src-tauri" / "tests" / "fixtures" / "corpus-manifest.json"


def build_stream_object_bytes(obj_id: int, dict_entries: str, stream_bytes: bytes) -> bytes:
    """Constructs a PDF stream object byte array with exact stream /Length."""
    length = len(stream_bytes)
    dict_str = f"<< /Length {length} {dict_entries} >>" if dict_entries.strip() else f"<< /Length {length} >>"
    header = f"{obj_id} 0 obj\n{dict_str}\nstream\n".encode("latin1")
    footer = b"\nendstream\nendobj\n"
    return header + stream_bytes + footer


def build_pdf_bytes_from_raw_objects(raw_objects: list[bytes], trailer_extras: str = "") -> bytes:
    """Constructs a valid PDF 1.4 from raw byte objects with exact cross-reference table."""
    buf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]

    for obj in raw_objects:
        offsets.append(len(buf))
        buf.extend(obj)
        if not obj.endswith(b"\n"):
            buf.extend(b"\n")

    xref_offset = len(buf)
    buf.extend(f"xref\n0 {len(raw_objects) + 1}\n".encode("latin1"))
    buf.extend(b"0000000000 65535 f \n")
    for off in offsets[1:]:
        buf.extend(f"{off:010d} 00000 n \n".encode("latin1"))

    trailer = (
        f"trailer\n"
        f"<< /Size {len(raw_objects) + 1} /Root 1 0 R {trailer_extras} >>\n"
        f"startxref\n"
        f"{xref_offset}\n"
        f"%%EOF\n"
    )
    buf.extend(trailer.encode("latin1"))
    return bytes(buf)


def generate_release_smoke_pdf() -> bytes:
    """
    Generates release-smoke.pdf:
    - 2 pages (595x842 pt, A4)
    - Page 1: ASCII header, RELEASE_SMOKE_ALPHA_TOKEN, Turkish Unicode text, PAGE_MARKER_PAGE_1, gray stroked rect.
    - Page 2: ASCII header, RELEASE_SMOKE_BETA_TOKEN, CANONICAL VERIFICATION TARGET, PAGE_MARKER_PAGE_2, blue filled rect.
    - Self-contained Type 3 Turkish Unicode Font (/F2) with:
      - FontBBox, FontMatrix, CharProcs, Encoding /Differences, FirstChar/LastChar, Widths, Resources, ToUnicode CMap.
    """
    # Type 3 Font CharProcs (uncolored glyphs using d1)
    # Glyph widths:
    # Idotaccent: 350, dotlessi: 300, Scedilla: 600, scedilla: 500,
    # Gbreve: 650, gbreve: 550, Udieresis: 650, udieresis: 550,
    # Odieresis: 650, odieresis: 550, Ccedilla: 600, ccedilla: 500
    charprocs = {
        "Idotaccent": (
            9,
            b"350 0 0 0 350 850 d1\n"
            b"125 0 100 650 re f\n"
            b"125 720 100 100 re f",
        ),
        "dotlessi": (
            10,
            b"300 0 0 0 300 500 d1\n"
            b"100 0 100 500 re f",
        ),
        "Scedilla": (
            11,
            b"600 0 0 -200 600 700 d1\n"
            b"100 0 400 700 re f\n"
            b"250 -150 100 150 re f",
        ),
        "scedilla": (
            12,
            b"500 0 0 -200 500 500 d1\n"
            b"100 0 300 500 re f\n"
            b"200 -150 100 150 re f",
        ),
        "Gbreve": (
            13,
            b"650 0 0 0 650 850 d1\n"
            b"100 0 450 700 re f\n"
            b"200 730 250 80 re f",
        ),
        "gbreve": (
            14,
            b"550 0 0 -200 550 800 d1\n"
            b"100 -150 350 650 re f\n"
            b"175 720 200 80 re f",
        ),
        "Udieresis": (
            15,
            b"650 0 0 0 650 850 d1\n"
            b"100 0 450 700 re f\n"
            b"175 730 80 80 re f\n"
            b"395 730 80 80 re f",
        ),
        "udieresis": (
            16,
            b"550 0 0 0 550 700 d1\n"
            b"100 0 350 500 re f\n"
            b"150 540 60 60 re f\n"
            b"340 540 60 60 re f",
        ),
        "Odieresis": (
            17,
            b"650 0 0 0 650 850 d1\n"
            b"100 0 450 700 re f\n"
            b"175 730 80 80 re f\n"
            b"395 730 80 80 re f",
        ),
        "odieresis": (
            18,
            b"550 0 0 0 550 700 d1\n"
            b"100 0 350 500 re f\n"
            b"150 540 60 60 re f\n"
            b"340 540 60 60 re f",
        ),
        "Ccedilla": (
            19,
            b"600 0 0 -200 600 700 d1\n"
            b"100 0 400 700 re f\n"
            b"250 -150 100 150 re f",
        ),
        "ccedilla": (
            20,
            b"500 0 0 -200 500 500 d1\n"
            b"100 0 300 500 re f\n"
            b"200 -150 100 150 re f",
        ),
    }

    tounicode_cmap = (
        b"/CIDInit /ProcSet findresource begin\n"
        b"12 dict begin\n"
        b"begincmap\n"
        b"/CIDSystemInfo <<\n"
        b"  /Registry (Adobe)\n"
        b"  /Ordering (UCS)\n"
        b"  /Supplement 0\n"
        b">> def\n"
        b"/CMapName /Custom-ToUnicode def\n"
        b"/CMapType 2 def\n"
        b"1 begincodespacerange\n"
        b"<80> <8B>\n"
        b"endcodespacerange\n"
        b"12 beginbfchar\n"
        b"<80> <0130>\n"
        b"<81> <0131>\n"
        b"<82> <015E>\n"
        b"<83> <015F>\n"
        b"<84> <011E>\n"
        b"<85> <011F>\n"
        b"<86> <00DC>\n"
        b"<87> <00FC>\n"
        b"<88> <00D6>\n"
        b"<89> <00F6>\n"
        b"<8A> <00C7>\n"
        b"<8B> <00E7>\n"
        b"endbfchar\n"
        b"endcmap\n"
        b"CMapName currentdict /CMap defineresource pop\n"
        b"end\n"
        b"end\n"
    )

    page1_stream = (
        b"0.75 0.75 0.75 RG\n"
        b"40 40 515 762 re\n"
        b"S\n"
        b"BT\n"
        b"/F1 16 Tf\n"
        b"50 750 Td\n"
        b"(ANTIGRAVITY RELEASE SMOKE PAGE 1) Tj\n"
        b"/F1 12 Tf\n"
        b"0 -30 Td\n"
        b"(RELEASE_SMOKE_ALPHA_TOKEN) Tj\n"
        b"0 -40 Td\n"
        b"/F2 12 Tf (\x80) Tj /F1 12 Tf (stanbul, ) Tj /F2 12 Tf (\x8A) Tj /F1 12 Tf (ankaya, ) Tj "
        b"/F2 12 Tf (\x80) Tj /F1 12 Tf (zmir, Mu) Tj /F2 12 Tf (\x85) Tj /F1 12 Tf (la, ) Tj "
        b"/F2 12 Tf (\x85\x87\x83) Tj /F1 12 Tf (i) Tj /F2 12 Tf (\x89\x8B) Tj /F1 12 Tf (, ) Tj "
        b"/F2 12 Tf (\x80) Tj /F1 12 Tf (I) Tj /F2 12 Tf (\x81\x82\x83\x84\x85\x86\x87\x88\x89\x8A\x8B) Tj\n"
        b"0 -630 Td\n"
        b"/F1 10 Tf\n"
        b"(PAGE_MARKER_PAGE_1) Tj\n"
        b"ET\n"
    )

    page2_stream = (
        b"0.2 0.4 0.8 rg\n"
        b"50 650 495 50 re\n"
        b"f\n"
        b"BT\n"
        b"/F1 16 Tf\n"
        b"50 750 Td\n"
        b"(ANTIGRAVITY RELEASE SMOKE PAGE 2) Tj\n"
        b"/F1 12 Tf\n"
        b"0 -150 Td\n"
        b"(RELEASE_SMOKE_BETA_TOKEN) Tj\n"
        b"0 -30 Td\n"
        b"(CANONICAL VERIFICATION TARGET) Tj\n"
        b"0 -520 Td\n"
        b"/F1 10 Tf\n"
        b"(PAGE_MARKER_PAGE_2) Tj\n"
        b"ET\n"
    )

    type3_font_dict = (
        b"6 0 obj\n"
        b"<< /Type /Font\n"
        b"   /Subtype /Type3\n"
        b"   /FontBBox [0 -200 650 850]\n"
        b"   /FontMatrix [0.001 0 0 0.001 0 0]\n"
        b"   /CharProcs <<\n"
        b"     /Idotaccent 9 0 R\n"
        b"     /dotlessi 10 0 R\n"
        b"     /Scedilla 11 0 R\n"
        b"     /scedilla 12 0 R\n"
        b"     /Gbreve 13 0 R\n"
        b"     /gbreve 14 0 R\n"
        b"     /Udieresis 15 0 R\n"
        b"     /udieresis 16 0 R\n"
        b"     /Odieresis 17 0 R\n"
        b"     /odieresis 18 0 R\n"
        b"     /Ccedilla 19 0 R\n"
        b"     /ccedilla 20 0 R\n"
        b"   >>\n"
        b"   /Encoding <<\n"
        b"     /Type /Encoding\n"
        b"     /Differences [128 /Idotaccent /dotlessi /Scedilla /scedilla /Gbreve /gbreve /Udieresis /udieresis /Odieresis /odieresis /Ccedilla /ccedilla]\n"
        b"   >>\n"
        b"   /FirstChar 128\n"
        b"   /LastChar 139\n"
        b"   /Widths [350 300 600 500 650 550 650 550 650 550 600 500]\n"
        b"   /Resources << /ProcSet [/PDF /Text] >>\n"
        b"   /ToUnicode 21 0 R\n"
        b">>\n"
        b"endobj\n"
    )

    raw_objects = [
        # 1: Catalog
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        # 2: Pages
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n",
        # 3: Page 1
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R /F2 6 0 R >> /ProcSet [/PDF /Text] >> /Contents 7 0 R >>\nendobj\n",
        # 4: Page 2
        b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> /ProcSet [/PDF /Text] >> /Contents 8 0 R >>\nendobj\n",
        # 5: Font F1 (Helvetica)
        b"5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        # 6: Font F2 (Type 3 Turkish Unicode Font)
        type3_font_dict,
        # 7: Page 1 Contents
        build_stream_object_bytes(7, "", page1_stream),
        # 8: Page 2 Contents
        build_stream_object_bytes(8, "", page2_stream),
    ]

    # 9..20: CharProcs streams
    for name, (obj_id, glyph_bytes) in charprocs.items():
        raw_objects.append(build_stream_object_bytes(obj_id, "", glyph_bytes))

    # 21: ToUnicode CMap stream
    raw_objects.append(build_stream_object_bytes(21, "", tounicode_cmap))

    return build_pdf_bytes_from_raw_objects(raw_objects)


def generate_large_page_dimensions_pdf() -> bytes:
    """
    Generates large-page-dimensions.pdf:
    - 1 page (7200x7200 pt / 100x100 in)
    - Below MAX_PAGE_DIMENSION_POINTS (14400.0 pt)
    - Simple boundary box and center text
    """
    page_content = (
        b"0.5 0.5 0.5 RG\n"
        b"100 100 7000 7000 re\n"
        b"S\n"
        b"BT\n"
        b"/F1 72 Tf\n"
        b"2000 3600 Td\n"
        b"(LARGE PAGE DIMENSIONS 7200x7200) Tj\n"
        b"ET\n"
    )
    raw_objects = [
        # 1: Catalog
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        # 2: Pages
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        # 3: Page 1
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 7200 7200] /Resources << /Font << /F1 4 0 R >> /ProcSet [/PDF /Text] >> /Contents 5 0 R >>\nendobj\n",
        # 4: Font F1 (Helvetica)
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        # 5: Page Contents
        build_stream_object_bytes(5, "", page_content),
    ]
    return build_pdf_bytes_from_raw_objects(raw_objects)


FIXTURES: dict[str, tuple[str, callable]] = {
    "release-smoke.pdf": ("core-release-smoke", generate_release_smoke_pdf),
    "large-page-dimensions.pdf": ("core-large-page-dimensions", generate_large_page_dimensions_pdf),
}


def verify_determinism(target_dir: Path) -> bool:
    """
    Non-destructive verification:
    - Generates fixtures in memory.
    - Compares byte length and SHA-256 with files on disk.
    - Compares with corpus-manifest.json entries.
    - Never overwrites files on disk.
    """
    print("Verifying determinism and manifest integrity (non-destructive)...")
    all_ok = True

    if not MANIFEST_PATH.is_file():
        print(f"ERROR: Manifest file not found at {MANIFEST_PATH}")
        return False

    with open(MANIFEST_PATH, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    manifest_fixtures = {f["id"]: f for f in manifest.get("fixtures", [])}
    manifest_generators = manifest.get("generators", {})

    if "py-release-smoke-generator" not in manifest_generators:
        print("ERROR: Generator 'py-release-smoke-generator' missing from corpus-manifest.json")
        all_ok = False
    else:
        gen = manifest_generators["py-release-smoke-generator"]
        if gen.get("kind") != "committed_script":
            print("ERROR: Generator kind must be 'committed_script'")
            all_ok = False
        if gen.get("reproducibilityStatus") != "deterministic_by_construction":
            print("ERROR: Generator reproducibilityStatus must be 'deterministic_by_construction'")
            all_ok = False

    for filename, (fixture_id, generator_fn) in FIXTURES.items():
        file_path = target_dir / filename
        generated_bytes = generator_fn()
        gen_sha = hashlib.sha256(generated_bytes).hexdigest()
        gen_len = len(generated_bytes)

        if not file_path.is_file():
            print(f"ERROR: Committed fixture file missing on disk: {file_path}")
            all_ok = False
            continue

        disk_bytes = file_path.read_bytes()
        disk_sha = hashlib.sha256(disk_bytes).hexdigest()
        disk_len = len(disk_bytes)

        if disk_sha != gen_sha:
            print(f"ERROR: SHA-256 mismatch for {filename}:")
            print(f"  Disk:      {disk_sha} ({disk_len} bytes)")
            print(f"  Generated: {gen_sha} ({gen_len} bytes)")
            all_ok = False
        else:
            print(f"OK: {filename} disk matches generated ({disk_len} bytes, SHA: {disk_sha[:16]}...)")

        if fixture_id not in manifest_fixtures:
            print(f"ERROR: Fixture ID '{fixture_id}' not found in corpus-manifest.json")
            all_ok = False
        else:
            mf = manifest_fixtures[fixture_id]
            if mf.get("sha256") != gen_sha:
                print(f"ERROR: Manifest SHA-256 mismatch for {fixture_id}:")
                print(f"  Manifest:  {mf.get('sha256')}")
                print(f"  Generated: {gen_sha}")
                all_ok = False
            if mf.get("byteLength") != gen_len:
                print(f"ERROR: Manifest byteLength mismatch for {fixture_id}:")
                print(f"  Manifest:  {mf.get('byteLength')}")
                print(f"  Generated: {gen_len}")
                all_ok = False

    if all_ok:
        print("ALL VERIFICATIONS PASSED: Bit-for-bit determinism and manifest alignment confirmed.")
    else:
        print("VERIFICATION FAILED.")
    return all_ok


def write_fixtures(target_dir: Path) -> dict[str, tuple[str, str, int]]:
    """Writes fixtures to disk deterministically and returns metadata."""
    target_dir.mkdir(parents=True, exist_ok=True)
    results = {}
    for filename, (fixture_id, generator_fn) in FIXTURES.items():
        file_path = target_dir / filename
        data = generator_fn()
        file_path.write_bytes(data)
        sha = hashlib.sha256(data).hexdigest()
        size = len(data)
        results[filename] = (fixture_id, sha, size)
        print(f"Wrote {filename}: {size} bytes, SHA-256: {sha}")
    return results


def main() -> None:
    parser = argparse.ArgumentParser(description="Deterministic Release Smoke Fixture Generator")
    parser.add_argument("--output", type=Path, default=ROOT, help="Output directory for fixtures")
    parser.add_argument(
        "--verify-determinism",
        action="store_true",
        help="Non-destructively verify generated bytes against disk and manifest",
    )
    args = parser.parse_args()

    if args.verify_determinism:
        ok = verify_determinism(args.output)
        sys.exit(0 if ok else 1)
    else:
        write_fixtures(args.output)


if __name__ == "__main__":
    main()
