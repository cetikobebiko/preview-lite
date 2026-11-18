#!/usr/bin/env python3
"""Generates scanned-turkish.pdf and scanned-turkish.words.json.

Project-authored fixture for the searchable-PDF (OCR text layer) export. Pages:

0. scanned page, /Rotate 0
1. scanned page, /Rotate 90  (image stored rotated so the page displays upright)
2. scanned page, /Rotate 180
3. scanned page, /Rotate 270
4. scanned page, /Rotate 0 with a CropBox offset from the MediaBox
5. page with a real text layer (Helvetica) and no image: must be skipped (has_text)
6. blank page: must be skipped (no_image)

Every scanned page carries the same Turkish text rendered at 150 dpi as a grayscale,
Flate-compressed image. The words file lists, per scanned page, every word with its box
in display points measured from the top-left corner of the displayed (rotated, cropped)
page, so tests can place words without OCR and check that they cover the ink.

Requires Pillow and an Arial TrueType font (macOS: /System/Library/Fonts/Supplemental).
The image is rasterized text only; no font is embedded in the PDF.

Usage: python3 generate_scanned_turkish.py [output-dir]
"""

import json
import os
import sys
import zlib

from PIL import Image, ImageDraw, ImageFont

DPI = 150
PAGE_W_PT, PAGE_H_PT = 595, 842  # A4, whole points
PX = lambda pt: round(pt * DPI / 72)
W_PX, H_PX = PX(PAGE_W_PT), PX(PAGE_H_PT)
CROP = (36, 30, 561, 788)  # page 4 CropBox (x0, y0, x1, y1) in points
FONT_PATHS = [
    "/System/Library/Fonts/Supplemental/Arial.ttf",
    "/Library/Fonts/Arial.ttf",
]
TITLE = "Çalışma Raporu Güz Dönemi"
LINES = [
    "Bu belge taranmış sayfalarda Türkçe karakterlerin",
    "doğru tanınıp tanınmadığını sınamak için hazırlandı.",
    "Öğrenciler şu konuları inceledi: ışık, ölçüm, güneş.",
    "İstanbul, İzmir ve Muğla şubelerinden gelen çizelgeler",
    "üç aşamada değerlendirildi; şoför çağrı öğüt ılık.",
]


def font(size):
    for path in FONT_PATHS:
        if os.path.exists(path):
            return ImageFont.truetype(path, size)
    sys.exit("Arial.ttf not found")


def upright_page():
    """The page as displayed: grayscale image and word boxes in image pixels."""
    image = Image.new("L", (W_PX, H_PX), 255)
    draw = ImageDraw.Draw(image)
    words = []
    y = PX(90)
    for text, size, advance in [(TITLE, 36, 70)] + [(line, 24, 46) for line in LINES]:
        face = font(size)
        x = PX(80)
        for word in text.split(" "):
            left, top, right, bottom = draw.textbbox((x, y), word, font=face)
            draw.text((x, y), word, font=face, fill=0)
            words.append({"text": word, "box": [left, top, right, bottom]})
            x = right + round(size * 0.35)
        y += advance
    return image, words


def stored(image, rotation):
    """The image as stored in user space so that /Rotate `rotation` displays it upright."""
    # PDF /Rotate turns the page clockwise for display; store it turned counter-clockwise.
    if rotation == 90:
        return image.transpose(Image.Transpose.ROTATE_90)
    if rotation == 180:
        return image.transpose(Image.Transpose.ROTATE_180)
    if rotation == 270:
        return image.transpose(Image.Transpose.ROTATE_270)
    return image


class Writer:
    def __init__(self):
        self.objects = []

    def add(self, body):
        self.objects.append(body)
        return len(self.objects)

    def stream(self, header, data):
        return self.add(header + b" /Length %d >>\nstream\n" % len(data) + data + b"\nendstream")

    def build(self, root):
        out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for number, body in enumerate(self.objects, start=1):
            offsets.append(len(out))
            out += b"%d 0 obj\n" % number + body + b"\nendobj\n"
        xref = len(out)
        out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(self.objects) + 1)
        for offset in offsets:
            out += b"%010d 00000 n \n" % offset
        out += b"trailer\n<< /Size %d /Root %d 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
            len(self.objects) + 1,
            root,
            xref,
        )
        return bytes(out)


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.dirname(os.path.abspath(__file__))
    image, words = upright_page()
    writer = Writer()
    pages_id = 1
    writer.add(b"")  # placeholder for the page tree
    helvetica = writer.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    page_ids = []
    words_out = []
    for index, rotation in enumerate([0, 90, 180, 270, 0]):
        picture = stored(image, rotation)
        width_px, height_px = picture.size
        width_pt, height_pt = (PAGE_W_PT, PAGE_H_PT) if rotation in (0, 180) else (PAGE_H_PT, PAGE_W_PT)
        data = zlib.compress(picture.tobytes(), 9)
        xobject = writer.stream(
            b"<< /Type /XObject /Subtype /Image /Width %d /Height %d /ColorSpace /DeviceGray"
            b" /BitsPerComponent 8 /Filter /FlateDecode" % (width_px, height_px),
            data,
        )
        content = writer.stream(b"<<", b"q %d 0 0 %d 0 0 cm /Im0 Do Q" % (width_pt, height_pt))
        crop = b""
        offset = (0.0, 0.0)
        if index == 4:
            crop = b" /CropBox [%d %d %d %d]" % CROP
            offset = (CROP[0], PAGE_H_PT - CROP[3])
        page_ids.append(
            writer.add(
                b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d]%s /Rotate %d"
                b" /Resources << /XObject << /Im0 %d 0 R >> >> /Contents %d 0 R >>"
                % (pages_id, width_pt, height_pt, crop, rotation, xobject, content)
            )
        )
        scale = 72 / DPI
        words_out.append(
            {
                "pageIndex": index,
                "words": [
                    {
                        "text": word["text"],
                        "box": [
                            round(word["box"][0] * scale - offset[0], 3),
                            round(word["box"][1] * scale - offset[1], 3),
                            round(word["box"][2] * scale - offset[0], 3),
                            round(word["box"][3] * scale - offset[1], 3),
                        ],
                    }
                    for word in words
                ],
            }
        )
    text = b"BT /F1 14 Tf 72 760 Td (Bu sayfada zaten bir metin katmani var.) Tj ET"
    text_content = writer.stream(b"<<", text)
    page_ids.append(
        writer.add(
            b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d] /Resources << /Font << /F1 %d 0 R >> >>"
            b" /Contents %d 0 R >>" % (pages_id, PAGE_W_PT, PAGE_H_PT, helvetica, text_content)
        )
    )
    blank_content = writer.stream(b"<<", b"")
    page_ids.append(
        writer.add(
            b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 %d %d] /Resources << >> /Contents %d 0 R >>"
            % (pages_id, PAGE_W_PT, PAGE_H_PT, blank_content)
        )
    )
    kids = b" ".join(b"%d 0 R" % page for page in page_ids)
    writer.objects[pages_id - 1] = b"<< /Type /Pages /Kids [%s] /Count %d >>" % (kids, len(page_ids))
    root = writer.add(b"<< /Type /Catalog /Pages %d 0 R >>" % pages_id)
    with open(os.path.join(out_dir, "scanned-turkish.pdf"), "wb") as handle:
        handle.write(writer.build(root))
    with open(os.path.join(out_dir, "scanned-turkish.words.json"), "w", encoding="utf-8") as handle:
        json.dump(
            {"dpi": DPI, "scannedPages": words_out, "textPage": 5, "blankPage": 6},
            handle,
            ensure_ascii=False,
            indent=1,
        )
        handle.write("\n")


if __name__ == "__main__":
    main()
