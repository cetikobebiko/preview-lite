#!/usr/bin/env python3
"""
Deterministic large-PDF generator for viewer and PDFium worker performance measurement.

The pages are deliberately expensive to render, so scrolling, zooming, render
cancellation and memory can be measured on a release build with documents of
thousands of pages (worker metrics: scripts/worker-metrics-report.mjs).

Page kinds:
  text    dense Helvetica text (text layer, search)
  image   a "scanned" 8-bit grayscale page image with an invisible text layer, like an
          OCR'd scan (image decode cost, memory pressure)
  vector  thousands of stroked curves plus translucent fills (path rasterisation cost)

Every page carries the marker PERF-P<number>. Every 50th page that is a text or image
page also carries the word "needle" for search measurements; vector pages hold only the
header line, so a needle page that is a vector page has none (mixed-1000: 19 matches,
heavy-400: 5).

These files are measurement inputs, not corpus fixtures: they are written to
tmp/perf-fixtures/ (outside src-tauri/tests/fixtures) and are never committed.
Standard library only. The same arguments give the same bytes with the same zlib
version; manifest.json records the SHA-256 of each file.

Usage:
  python3 scripts/generate-perf-fixtures.py                      # every preset
  python3 scripts/generate-perf-fixtures.py --preset mixed-1000
  python3 scripts/generate-perf-fixtures.py --pages 200 --mix text:1,vector:1 --out x.pdf
"""

import argparse
import hashlib
import json
import random
import sys
import zlib
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT_DIR = REPO_ROOT / "tmp" / "perf-fixtures"
GENERATOR_VERSION = 1

PAGE_SIZES = {
    "a4": (595.276, 841.89),
    "a4l": (841.89, 595.276),
    "a3": (841.89, 1190.551),
    "letter": (612.0, 792.0),
}

PRESETS = {
    "text-1000": {"pages": 1000, "mix": "text:1", "sizes": "a4", "rotate_every": 0},
    "text-3000": {"pages": 3000, "mix": "text:1", "sizes": "a4", "rotate_every": 0},
    "heavy-400": {"pages": 400, "mix": "image:1,vector:1", "sizes": "a4", "rotate_every": 0},
    "mixed-1000": {"pages": 1000, "mix": "text:8,image:1,vector:1", "sizes": "a4,a4l,a3,letter", "rotate_every": 7},
}

WORDS = (
    "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor "
    "incididunt ut labore et dolore magna aliqua enim ad minim veniam quis nostrud "
    "exercitation ullamco laboris nisi aliquip ex ea commodo consequat duis aute irure "
    "reprehenderit voluptate velit esse cillum fugiat nulla pariatur excepteur sint "
    "occaecat cupidatat non proident sunt culpa qui officia deserunt mollit anim id est "
    "laborum document session render worker generation snapshot geometry viewport "
    "scroll anchor thumbnail outline annotation redaction signature prepress channel "
    "measurement latency cancellation threshold buffer canvas layer spread margin"
).split()

FONT_SIZE = 9.5
LEADING = 13.0
MARGIN = 54.0
# Helvetica's average advance for lower-case text is about half the font size.
AVERAGE_CHAR_WIDTH = FONT_SIZE * 0.5


def parse_mix(text):
    mix = []
    for part in text.split(","):
        kind, _, weight = part.strip().partition(":")
        if kind not in ("text", "image", "vector"):
            raise ValueError(f"unknown page kind: {kind}")
        weight = int(weight or "1")
        if weight > 0:
            mix.append((kind, weight))
    if not mix:
        raise ValueError("--mix needs at least one page kind with a positive weight")
    return mix


def parse_sizes(text):
    sizes = [name.strip() for name in text.split(",") if name.strip()]
    unknown = [name for name in sizes if name not in PAGE_SIZES]
    if unknown or not sizes:
        raise ValueError(f"unknown page sizes: {unknown or text}")
    return sizes


def plan_pages(pages, mix, sizes, rotate_every, seed):
    """Kind, size and /Rotate of every page. Sizes come in runs, as in real documents."""
    rng = random.Random(f"{seed}:plan")
    kinds = [kind for kind, _ in mix]
    weights = [weight for _, weight in mix]
    plan = []
    size = sizes[0]
    run_left = 0
    for index in range(pages):
        if len(sizes) > 1 and run_left == 0:
            size = rng.choice(sizes)
            run_left = rng.randint(1, 12)
        run_left -= 1
        rotate = 90 if rotate_every > 0 and (index + 1) % rotate_every == 0 else 0
        plan.append({"kind": rng.choices(kinds, weights)[0], "size": size, "rotate": rotate})
    return plan


def text_lines(rng, page_number, page_count, width_pt, height_pt):
    chars_per_line = max(20, int((width_pt - 2 * MARGIN) / AVERAGE_CHAR_WIDTH))
    line_count = max(1, int((height_pt - 2 * MARGIN) / LEADING) - 1)
    lines = [f"Sayfa {page_number} / {page_count}  PERF-P{page_number:05d}"]
    for line_index in range(line_count):
        words = []
        length = 0
        while length < chars_per_line:
            word = rng.choice(WORDS)
            words.append(word)
            length += len(word) + 1
        if page_number % 50 == 0 and line_index == line_count // 2:
            words[len(words) // 2] = "needle"
        lines.append(" ".join(words)[:chars_per_line])
    return lines


def text_operators(lines, height_pt, render_mode=0):
    out = [f"BT /F1 {FONT_SIZE} Tf {LEADING} TL {render_mode} Tr {MARGIN} {height_pt - MARGIN - FONT_SIZE:.2f} Td"]
    for line in lines:
        out.append(f"({line}) Tj T*")
    out.append("ET")
    return "\n".join(out)


class ScanRows:
    """Row pools for synthetic scans: paper with grain, and rows cut through text lines."""

    def __init__(self, width, seed):
        rng = random.Random(f"{seed}:scan:{width}")
        left = int(width * 0.08)
        right = width - left
        self.paper = [self._row(rng, width, 0, width, ink=False) for _ in range(24)]
        self.ink = [self._row(rng, width, left, right, ink=True) for _ in range(192)]

    @staticmethod
    def _row(rng, width, start, end, ink):
        row = bytearray()
        while len(row) < start:
            row += bytes([rng.randint(238, 250)]) * rng.randint(4, 16)
        while len(row) < end:
            if ink and rng.random() < 0.45:
                row += bytes([rng.randint(20, 90)]) * rng.randint(1, 4)
            elif ink and rng.random() < 0.05:
                row += bytes([rng.randint(240, 250)]) * rng.randint(8, 20)
            else:
                row += bytes([rng.randint(232, 250)]) * rng.randint(1, 6)
        while len(row) < width:
            row += bytes([rng.randint(238, 250)]) * rng.randint(4, 16)
        return bytes(row[:width])


def scan_image(rows, rng, width, height):
    text_rows = 18
    gap_rows = 11
    top = int(height * 0.07)
    bottom = height - top
    out = []
    y = 0
    while y < height:
        if top <= y < bottom - text_rows and (y - top) % (text_rows + gap_rows) == 0 and rng.random() > 0.04:
            start = rng.randrange(len(rows.ink) - text_rows)
            out.extend(rows.ink[start:start + text_rows])
            y += text_rows
        else:
            out.append(rows.paper[rng.randrange(len(rows.paper))])
            y += 1
    return b"".join(out[:height])


def vector_operators(rng, width_pt, height_pt, paths=600, segments=8, fills=150):
    out = ["q 0.6 w 1 J 1 j"]
    for _ in range(fills):
        x, y = rng.uniform(0, width_pt), rng.uniform(0, height_pt)
        out.append(f"/GS0 gs {rng.random():.2f} {rng.random():.2f} {rng.random():.2f} rg {x:.1f} {y:.1f} m")
        for _ in range(rng.randint(4, 9)):
            x = min(max(x + rng.uniform(-90, 90), 0), width_pt)
            y = min(max(y + rng.uniform(-90, 90), 0), height_pt)
            out.append(f"{x:.1f} {y:.1f} l")
        out.append("h f")
    out.append("/GS1 gs")
    for _ in range(paths):
        x, y = rng.uniform(0, width_pt), rng.uniform(0, height_pt)
        out.append(f"{rng.uniform(0.2, 1.8):.2f} w {rng.random():.2f} {rng.random():.2f} {rng.random():.2f} RG {x:.1f} {y:.1f} m")
        for _ in range(segments):
            points = []
            for _ in range(3):
                x = min(max(x + rng.uniform(-40, 40), 0), width_pt)
                y = min(max(y + rng.uniform(-40, 40), 0), height_pt)
                points.append(f"{x:.1f} {y:.1f}")
            out.append(" ".join(points) + " c")
        out.append("S")
    out.append("Q")
    return "\n".join(out)


class PdfWriter:
    def __init__(self, handle):
        self.handle = handle
        self.offset = 0
        self.offsets = {}
        self._write(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")

    def _write(self, data):
        self.handle.write(data)
        self.offset += len(data)

    def object(self, obj_id, body):
        self.offsets[obj_id] = self.offset
        self._write(f"{obj_id} 0 obj\n{body}\nendobj\n".encode("latin-1"))

    def stream(self, obj_id, entries, data, compress=True):
        if compress:
            data = zlib.compress(data, 6)
            entries = f"{entries} /Filter /FlateDecode"
        self.offsets[obj_id] = self.offset
        self._write(f"{obj_id} 0 obj\n<< /Length {len(data)} {entries} >>\nstream\n".encode("latin-1"))
        self._write(data)
        self._write(b"\nendstream\nendobj\n")

    def finish(self, root_id, document_id):
        size = max(self.offsets) + 1
        xref_offset = self.offset
        rows = [f"xref\n0 {size}\n", "0000000000 65535 f \n"]
        for obj_id in range(1, size):
            rows.append(f"{self.offsets[obj_id]:010d} 00000 n \n")
        rows.append(
            f"trailer\n<< /Size {size} /Root {root_id} 0 R /ID [<{document_id}> <{document_id}>] >>\n"
            f"startxref\n{xref_offset}\n%%EOF\n"
        )
        self._write("".join(rows).encode("latin-1"))


def generate(out_path, pages, mix, sizes, rotate_every, dpi, seed):
    params = {
        "generatorVersion": GENERATOR_VERSION, "pages": pages, "mix": mix, "sizes": sizes,
        "rotateEvery": rotate_every, "dpi": dpi, "seed": seed,
    }
    plan = plan_pages(pages, parse_mix(mix), parse_sizes(sizes), rotate_every, seed)
    document_id = hashlib.sha256(json.dumps(params, sort_keys=True).encode()).hexdigest()[:32]

    catalog_id, root_pages_id, font_id, fill_state_id, stroke_state_id = 1, 2, 3, 4, 5
    kids_per_node = 100
    node_count = (pages + kids_per_node - 1) // kids_per_node
    node_ids = [6 + index for index in range(node_count)]
    next_id = 6 + node_count
    page_objects = []
    for entry in plan:
        ids = {"page": next_id, "content": next_id + 1}
        next_id += 2
        if entry["kind"] == "image":
            ids["image"] = next_id
            next_id += 1
        page_objects.append(ids)

    scan_rows = {}
    counts = {"text": 0, "image": 0, "vector": 0}
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "wb") as handle:
        writer = PdfWriter(handle)
        writer.object(catalog_id, f"<< /Type /Catalog /Pages {root_pages_id} 0 R >>")
        writer.object(root_pages_id, f"<< /Type /Pages /Kids [{' '.join(f'{node} 0 R' for node in node_ids)}] /Count {pages} >>")
        writer.object(font_id, "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
        writer.object(fill_state_id, "<< /Type /ExtGState /ca 0.35 >>")
        writer.object(stroke_state_id, "<< /Type /ExtGState /ca 1 >>")
        for node_index, node_id in enumerate(node_ids):
            first = node_index * kids_per_node
            kids = [ids["page"] for ids in page_objects[first:first + kids_per_node]]
            writer.object(node_id, f"<< /Type /Pages /Parent {root_pages_id} 0 R /Kids [{' '.join(f'{kid} 0 R' for kid in kids)}] /Count {len(kids)} >>")

        for index, (entry, ids) in enumerate(zip(plan, page_objects)):
            page_number = index + 1
            width_pt, height_pt = PAGE_SIZES[entry["size"]]
            rng = random.Random(f"{seed}:page:{page_number}")
            lines = text_lines(rng, page_number, pages, width_pt, height_pt)
            resources = f"/Font << /F1 {font_id} 0 R >>"
            kind = entry["kind"]
            counts[kind] += 1
            if kind == "text":
                content = text_operators(lines, height_pt)
            elif kind == "image":
                image_width = max(1, round(width_pt / 72 * dpi))
                image_height = max(1, round(height_pt / 72 * dpi))
                rows = scan_rows.get(image_width)
                if rows is None:
                    rows = scan_rows[image_width] = ScanRows(image_width, seed)
                writer.stream(
                    ids["image"],
                    f"/Type /XObject /Subtype /Image /Width {image_width} /Height {image_height} /ColorSpace /DeviceGray /BitsPerComponent 8",
                    scan_image(rows, rng, image_width, image_height),
                )
                resources += f" /XObject << /Im0 {ids['image']} 0 R >>"
                content = f"q {width_pt} 0 0 {height_pt} 0 0 cm /Im0 Do Q\n" + text_operators(lines, height_pt, render_mode=3)
            else:
                resources += f" /ExtGState << /GS0 {fill_state_id} 0 R /GS1 {stroke_state_id} 0 R >>"
                content = vector_operators(rng, width_pt, height_pt) + "\n" + text_operators(lines[:1], height_pt)
            writer.stream(ids["content"], "", content.encode("latin-1"))
            node_id = node_ids[index // kids_per_node]
            rotate = f" /Rotate {entry['rotate']}" if entry["rotate"] else ""
            writer.object(
                ids["page"],
                f"<< /Type /Page /Parent {node_id} 0 R /MediaBox [0 0 {width_pt} {height_pt}]{rotate} "
                f"/Resources << {resources} >> /Contents {ids['content']} 0 R >>",
            )
        writer.finish(catalog_id, document_id)

    digest = hashlib.sha256(out_path.read_bytes()).hexdigest()
    return {
        "file": out_path.name, "bytes": out_path.stat().st_size, "sha256": digest,
        "kinds": counts,
        "sizes": {name: sum(1 for entry in plan if entry["size"] == name) for name in parse_sizes(sizes)},
        "rotated": sum(1 for entry in plan if entry["rotate"]),
        "params": params,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--preset", action="append", choices=sorted(PRESETS), help="preset to build (repeatable; default: all)")
    parser.add_argument("--out-dir", type=Path, default=DEFAULT_OUT_DIR)
    parser.add_argument("--pages", type=int, help="custom document: page count")
    parser.add_argument("--mix", default="text:1", help="custom document: kind weights, e.g. text:8,image:1,vector:1")
    parser.add_argument("--sizes", default="a4", help=f"custom document: {','.join(PAGE_SIZES)}")
    parser.add_argument("--rotate-every", type=int, default=0, help="custom document: /Rotate 90 on every Nth page")
    parser.add_argument("--out", type=Path, help="custom document: output path")
    parser.add_argument("--dpi", type=int, default=150, help="scan image resolution")
    parser.add_argument("--seed", default="antigravity-perf-v1")
    args = parser.parse_args(argv)

    jobs = []
    if args.pages is not None:
        if args.preset or not args.out or args.pages < 1:
            parser.error("a custom document needs --pages >= 1 and --out, without --preset")
        jobs.append((args.out, {"pages": args.pages, "mix": args.mix, "sizes": args.sizes, "rotate_every": args.rotate_every}))
    else:
        for name in args.preset or sorted(PRESETS):
            jobs.append((args.out_dir / f"perf-{name}.pdf", PRESETS[name]))

    results = []
    for out_path, spec in jobs:
        print(f"generating {out_path} ...", file=sys.stderr, flush=True)
        result = generate(out_path, spec["pages"], spec["mix"], spec["sizes"], spec["rotate_every"], args.dpi, args.seed)
        print(f"  {result['bytes'] / 1048576:.1f} MB  {result['kinds']}", file=sys.stderr)
        results.append(result)

    if args.pages is None:
        manifest = {
            "generatorVersion": GENERATOR_VERSION,
            "python": sys.version.split()[0],
            "zlib": zlib.ZLIB_RUNTIME_VERSION,
            "files": results,
        }
        (args.out_dir / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
