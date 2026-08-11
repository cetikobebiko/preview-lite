#!/usr/bin/env python3
"""Build the deterministic, synthetic PDF sanitization characterization corpus.

The generator intentionally uses only the Python standard library. The files are
test evidence, not production sanitization inputs or a second PDF engine.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import struct
from pathlib import Path


ROOT = Path(__file__).resolve().parent
FIXED_ID = b"SANITIZE-FIX-ID"


def stream(obj_id: int, payload: bytes, entries: str = "") -> bytes:
    suffix = f" {entries.strip()}" if entries.strip() else ""
    return (
        f"{obj_id} 0 obj\n<< /Length {len(payload)}{suffix} >>\nstream\n".encode("latin1")
        + payload
        + b"\nendstream\nendobj\n"
    )


def classic_pdf(objects: list[bytes], trailer: str = "", version: str = "1.7") -> bytes:
    output = bytearray(f"%PDF-{version}\n%\xe2\xe3\xcf\xd3\n".encode("latin1"))
    offsets = [0]
    for item in objects:
        offsets.append(len(output))
        output.extend(item)
        if not item.endswith(b"\n"):
            output.extend(b"\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(objects) + 1}\n".encode("latin1"))
    output.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        output.extend(f"{offset:010d} 00000 n \n".encode("latin1"))
    output.extend(
        (
            f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R {trailer} >>\n"
            f"startxref\n{xref}\n%%EOF\n"
        ).encode("latin1")
    )
    return bytes(output)


def page_base(catalog_extra: str = "", page_extra: str = "", extra: list[bytes] | None = None) -> bytes:
    objects = [
        f"1 0 obj\n<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>\nendobj\n".encode("latin1"),
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        f"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R {page_extra} >>\nendobj\n".encode("latin1"),
        stream(4, b"q 0.95 g 0 0 300 300 re f Q"),
    ]
    objects.extend(extra or [])
    return classic_pdf(objects)


def clean() -> bytes:
    return page_base()


def metadata() -> bytes:
    image = stream(
        8,
        b"\x7f",
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray "
        "/BitsPerComponent 8 /Metadata 9 0 R",
    )
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R /PieceInfo << /Fixture << /LastModified (D:20000101000000Z) >> >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Metadata 7 0 R /Resources << /XObject << /Im1 8 0 R >> >> >>\nendobj\n",
        stream(4, b"q 10 0 0 10 10 10 cm /Im1 Do Q"),
        b"5 0 obj\n<< /Title (SANITIZATION_INFO_PAYLOAD_001) /Author (Fixture Author) >>\nendobj\n",
        stream(6, b"<x:xmpmeta>SANITIZATION_XMP_PAYLOAD_001</x:xmpmeta>", "/Type /Metadata /Subtype /XML"),
        stream(7, b"<x:xmpmeta>SANITIZATION_PAGE_XMP_PAYLOAD_001</x:xmpmeta>", "/Type /Metadata /Subtype /XML"),
        image,
        stream(9, b"<x:xmpmeta>SANITIZATION_OBJECT_XMP_PAYLOAD_001</x:xmpmeta>", "/Type /Metadata /Subtype /XML"),
    ]
    fixed_hex = FIXED_ID.hex().upper()
    return classic_pdf(objects, f"/Info 5 0 R /ID [<{fixed_hex}> <{fixed_hex}>]")


def javascript_actions() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Names 5 0 R /OpenAction 9 0 R /AA << /WC 10 0 R >> /AcroForm 14 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /AA << /O 11 0 R /C << /S /JavaScript /JS <53414E4954495A4154494F4E5F4A535F4845585F303031> >> >> /Annots [12 0 R 15 0 R] >>\nendobj\n",
        stream(4, b"q Q"),
        b"5 0 obj\n<< /Dests 6 0 R /JavaScript 7 0 R >>\nendobj\n",
        b"6 0 obj\n<< /Names [(kept-destination) [3 0 R /Fit]] >>\nendobj\n",
        b"7 0 obj\n<< /Names [(fixture-script) 8 0 R] >>\nendobj\n",
        b"8 0 obj\n<< /S /JavaScript /JS (SANITIZATION_JS_PAYLOAD_001) >>\nendobj\n",
        b"9 0 obj\n<< /S /JavaScript /JS 17 0 R >>\nendobj\n",
        b"10 0 obj\n<< /S /JavaScript /JS (SANITIZATION_CATALOG_AA_001) >>\nendobj\n",
        b"11 0 obj\n<< /S /JavaScript /JS (SANITIZATION_PAGE_AA_001) >>\nendobj\n",
        b"12 0 obj\n<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A 13 0 R /AA << /E 10 0 R >> >>\nendobj\n",
        b"13 0 obj\n<< /S /JavaScript /JS (SANITIZATION_ANNOTATION_JS_001) >>\nendobj\n",
        b"14 0 obj\n<< /Fields [15 0 R] /CO [15 0 R] >>\nendobj\n",
        b"15 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (fixture-field) /Rect [20 20 80 40] /A 13 0 R /AA << /K 13 0 R /V 13 0 R /C 13 0 R /F 13 0 R >> >>\nendobj\n",
        b"16 0 obj\nnull\nendobj\n",
        stream(17, b"SANITIZATION_JS_STREAM_PAYLOAD_001"),
    ]
    return classic_pdf(objects)


def combined_names() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Names 5 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>\nendobj\n",
        stream(4, b"q Q"),
        b"5 0 obj\n<< /Dests 6 0 R /JavaScript 7 0 R /EmbeddedFiles 9 0 R >>\nendobj\n",
        b"6 0 obj\n<< /Names [(preserved-destination) [3 0 R /Fit]] >>\nendobj\n",
        b"7 0 obj\n<< /Names [(fixture-script) 8 0 R] >>\nendobj\n",
        b"8 0 obj\n<< /S /JavaScript /JS (SANITIZATION_COMBINED_NAMES_JS_001) >>\nendobj\n",
        b"9 0 obj\n<< /Names [(fixture.bin) 10 0 R] >>\nendobj\n",
        b"10 0 obj\n<< /Type /Filespec /F (fixture.bin) /EF << /F 11 0 R >> >>\nendobj\n",
        stream(11, b"SANITIZATION_COMBINED_NAMES_EMBEDDED_001", "/Type /EmbeddedFile"),
    ]
    return classic_pdf(objects)


def openaction_destination() -> bytes:
    return page_base(catalog_extra="/OpenAction [3 0 R /Fit]")


def action_chain_network() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /OpenAction 5 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Annots [12 0 R] >>\nendobj\n",
        stream(4, b"q Q"),
        b"5 0 obj\n<< /S /Launch /F 6 0 R /Win << /F (fixture.exe) >> /Next [7 0 R 8 0 R] >>\nendobj\n",
        b"6 0 obj\n<< /Type /Filespec /F (fixture.exe) >>\nendobj\n",
        b"7 0 obj\n<< /S /URI /URI (https://example.invalid/) /Next 9 0 R >>\nendobj\n",
        b"8 0 obj\n<< /S /GoToR /F 6 0 R /D (remote-destination) /Next 10 0 R >>\nendobj\n",
        b"9 0 obj\n<< /S /SubmitForm /F (https://example.invalid/submit) /Next 11 0 R >>\nendobj\n",
        b"10 0 obj\n<< /S /GoToE /F 6 0 R /D [0 /Fit] /Next 5 0 R >>\nendobj\n",
        b"11 0 obj\n<< /S /ImportData /F 6 0 R >>\nendobj\n",
        b"12 0 obj\n<< /Type /Annot /Subtype /Link /Rect [0 0 10 10] /A 7 0 R >>\nendobj\n",
    ]
    return classic_pdf(objects)


def embedded_files() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 5 0 R >> /AF [7 0 R] /Collection << /View /D >> /StructTreeRoot 10 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /AF [7 0 R] /Annots [9 0 R] >>\nendobj\n",
        stream(4, b"q Q"),
        b"5 0 obj\n<< /Names [(fixture.bin) 7 0 R] >>\nendobj\n",
        b"6 0 obj\nnull\nendobj\n",
        b"7 0 obj\n<< /Type /Filespec /F (fixture.bin) /UF (fixture.bin) /Desc (fixture attachment) /EF << /F 8 0 R >> /RF << /F 8 0 R >> /Params << /Size 37 /CreationDate (D:20000101000000Z) >> >>\nendobj\n",
        stream(8, b"SANITIZATION_EMBEDDED_PAYLOAD_001", "/Type /EmbeddedFile"),
        b"9 0 obj\n<< /Type /Annot /Subtype /FileAttachment /Rect [0 0 10 10] /FS 7 0 R /AF [7 0 R] >>\nendobj\n",
        b"10 0 obj\n<< /Type /StructTreeRoot /K 11 0 R >>\nendobj\n",
        b"11 0 obj\n<< /Type /StructElem /S /Document /P 10 0 R /Pg 3 0 R /K 0 /AF [7 0 R << /Type /Filespec /F (structure-direct.bin) /EF << /F 12 0 R >> >>] >>\nendobj\n",
        stream(12, b"SANITIZATION_STRUCTURE_DIRECT_EMBEDDED_001", "/Type /EmbeddedFile"),
    ]
    return classic_pdf(objects)


def xfa_single_stream() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] /XFA 5 0 R >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>\nendobj\n",
        stream(4, b"q Q"),
        stream(5, b"<xdp:xdp>SANITIZATION_XFA_SINGLE_STREAM_001</xdp:xdp>"),
    ]
    return classic_pdf(objects)


def unsupported_active_content() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] /XFA [ (template) 6 0 R (datasets) 7 0 R ] >> /OCProperties << /OCGs [8 0 R] /D << /OFF [8 0 R] >> >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Resources << /Properties << /Layer1 8 0 R /Membership 15 0 R >> >> /Annots [9 0 R 11 0 R 12 0 R 13 0 R 14 0 R 16 0 R] >>\nendobj\n",
        stream(4, b"/OC /Layer1 BDC SANITIZATION_OPTIONAL_CONTENT_001 EMC /OC /Membership BDC SANITIZATION_OCMD_CONTENT_001 EMC"),
        b"5 0 obj\nnull\nendobj\n",
        stream(6, b"<template>SANITIZATION_XFA_PAYLOAD_001</template>"),
        stream(7, b"<datasets>SANITIZATION_XFA_DATA_001</datasets>"),
        b"8 0 obj\n<< /Type /OCG /Name (Hidden fixture layer) >>\nendobj\n",
        b"9 0 obj\n<< /Type /Annot /Subtype /RichMedia /Rect [0 0 10 10] /RichMediaContent << /Assets << /Names [(asset) 10 0 R] >> >> >>\nendobj\n",
        stream(10, b"SANITIZATION_RICHMEDIA_PAYLOAD_001", "/Type /EmbeddedFile"),
        b"11 0 obj\n<< /Type /Annot /Subtype /Screen /Rect [0 0 10 10] /A << /S /Rendition /R << /S /MR /C 10 0 R >> >> >>\nendobj\n",
        b"12 0 obj\n<< /Type /Annot /Subtype /Movie /Rect [0 0 10 10] /Movie << /F (movie.mov) >> >>\nendobj\n",
        b"13 0 obj\n<< /Type /Annot /Subtype /Sound /Rect [0 0 10 10] /Sound 10 0 R >>\nendobj\n",
        b"14 0 obj\n<< /Type /Annot /Subtype /3D /Rect [0 0 10 10] /3DD 10 0 R >>\nendobj\n",
        b"15 0 obj\n<< /Type /OCMD /OCGs [8 0 R] /P /AnyOn >>\nendobj\n",
        b"16 0 obj\n<< /Type /Annot /Subtype /Text /Rect [20 20 30 30] /Contents (Layer-associated note) /OC 15 0 R >>\nendobj\n",
    ]
    return classic_pdf(objects)


def incremental_revision() -> bytes:
    first = page_base(
        catalog_extra="/OpenAction 5 0 R",
        extra=[b"5 0 obj\n<< /S /JavaScript /JS (SANITIZATION_OLD_REVISION_PAYLOAD_001) >>\nendobj\n"],
    )
    previous_xref = int(first[first.rfind(b"startxref") :].splitlines()[1])
    catalog = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /OpenAction 5 0 R /Metadata 6 0 R >>\nendobj\n"
    metadata_stream = stream(6, b"SANITIZATION_INCREMENTAL_XMP_001", "/Type /Metadata /Subtype /XML")
    off1 = len(first)
    off6 = off1 + len(catalog)
    xref = off6 + len(metadata_stream)
    update = bytearray(catalog + metadata_stream)
    update.extend(f"xref\n1 1\n{off1:010d} 00000 n \n6 1\n{off6:010d} 00000 n \n".encode("latin1"))
    update.extend(
        f"trailer\n<< /Size 7 /Root 1 0 R /Prev {previous_xref} >>\nstartxref\n{xref}\n%%EOF\n".encode("latin1")
    )
    return first + bytes(update)


def object_xref_stream() -> bytes:
    compressed = b"<< /Type /Metadata /Marker (SANITIZATION_OBJECT_STREAM_PAYLOAD_001) >>"
    obj_stream_payload = f"5 0\n".encode("latin1") + compressed
    output = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
    offsets: dict[int, int] = {}
    for obj_id, body in [
        (1, b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>\nendobj\n"),
        (2, b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n"),
        (3, b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>\nendobj\n"),
        (4, stream(4, b"q Q")),
        (6, stream(6, obj_stream_payload, "/Type /ObjStm /N 1 /First 4")),
    ]:
        offsets[obj_id] = len(output)
        output.extend(body)
    offsets[7] = len(output)
    entries = bytearray()
    for obj_id in range(8):
        if obj_id == 0:
            entries.extend(b"\x00" + (0).to_bytes(4, "big") + (65535).to_bytes(2, "big"))
        elif obj_id == 5:
            entries.extend(b"\x02" + (6).to_bytes(4, "big") + (0).to_bytes(2, "big"))
        else:
            entries.extend(b"\x01" + offsets[obj_id].to_bytes(4, "big") + (0).to_bytes(2, "big"))
    output.extend(stream(7, bytes(entries), "/Type /XRef /Size 8 /W [1 4 2] /Root 1 0 R"))
    output.extend(f"startxref\n{offsets[7]}\n%%EOF\n".encode("latin1"))
    return bytes(output)


def incremental_xref_stream_chain() -> bytes:
    output = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
    offsets: dict[int, int] = {}
    revision_one = [
        (1, b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>\nendobj\n"),
        (2, b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n"),
        (3, b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>\nendobj\n"),
        (4, stream(4, b"q Q")),
        (5, stream(5, b"SANITIZATION_XREF_STREAM_OLD_REVISION_001", "/Type /Metadata /Subtype /XML")),
    ]
    for obj_id, body in revision_one:
        offsets[obj_id] = len(output)
        output.extend(body)

    offsets[6] = len(output)
    first_entries = bytearray()
    for obj_id in range(7):
        if obj_id == 0:
            first_entries.extend(b"\x00" + (0).to_bytes(4, "big") + (65535).to_bytes(2, "big"))
        else:
            first_entries.extend(b"\x01" + offsets[obj_id].to_bytes(4, "big") + (0).to_bytes(2, "big"))
    output.extend(stream(6, bytes(first_entries), "/Type /XRef /Size 7 /W [1 4 2] /Root 1 0 R"))
    output.extend(f"startxref\n{offsets[6]}\n%%EOF\n".encode("latin1"))

    latest_catalog_offset = len(output)
    output.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R /PageMode /UseNone >>\nendobj\n")
    latest_metadata_offset = len(output)
    output.extend(stream(5, b"SANITIZATION_XREF_STREAM_LATEST_REVISION_001", "/Type /Metadata /Subtype /XML"))
    latest_xref_offset = len(output)
    second_entries = bytearray()
    for entry_type, offset, generation in [
        (0, 0, 65535),
        (1, latest_catalog_offset, 0),
        (1, latest_metadata_offset, 0),
        (1, latest_xref_offset, 0),
    ]:
        second_entries.extend(bytes([entry_type]) + offset.to_bytes(4, "big") + generation.to_bytes(2, "big"))
    output.extend(
        stream(
            7,
            bytes(second_entries),
            f"/Type /XRef /Size 8 /Index [0 2 5 1 7 1] /W [1 4 2] /Root 1 0 R /Prev {offsets[6]}",
        )
    )
    output.extend(f"startxref\n{latest_xref_offset}\n%%EOF\n".encode("latin1"))
    return bytes(output)


def hybrid_xref() -> bytes:
    base = page_base()
    previous_xref = int(base[base.rfind(b"startxref") :].splitlines()[1])
    compressed = b"<< /Type /Metadata /Marker (SANITIZATION_HYBRID_OBJECT_001) >>"
    header = b"5 0\n"
    obj6 = stream(6, header + compressed, f"/Type /ObjStm /N 1 /First {len(header)}")
    updated_catalog = b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /Metadata 5 0 R >>\nendobj\n"
    off1 = len(base)
    off6 = off1 + len(updated_catalog)
    off7 = off6 + len(obj6)
    xref_entries = (
        b"\x02" + (6).to_bytes(4, "big") + (0).to_bytes(2, "big")
        + b"\x01" + off6.to_bytes(4, "big") + (0).to_bytes(2, "big")
        + b"\x01" + off7.to_bytes(4, "big") + (0).to_bytes(2, "big")
    )
    obj7 = stream(7, xref_entries, "/Type /XRef /Size 8 /Index [5 3] /W [1 4 2] /Root 1 0 R")
    xref = off7 + len(obj7)
    update = bytearray(updated_catalog + obj6 + obj7)
    update.extend(f"xref\n1 1\n{off1:010d} 00000 n \n6 2\n{off6:010d} 00000 n \n{off7:010d} 00000 n \n".encode("latin1"))
    update.extend(
        f"trailer\n<< /Size 8 /Root 1 0 R /Prev {previous_xref} /XRefStm {off7} >>\nstartxref\n{xref}\n%%EOF\n".encode("latin1")
    )
    return base + bytes(update)


def rc4(key: bytes, data: bytes) -> bytes:
    state = list(range(256))
    j = 0
    for i in range(256):
        j = (j + state[i] + key[i % len(key)]) % 256
        state[i], state[j] = state[j], state[i]
    i = j = 0
    result = bytearray()
    for value in data:
        i = (i + 1) % 256
        j = (j + state[i]) % 256
        state[i], state[j] = state[j], state[i]
        result.append(value ^ state[(state[i] + state[j]) % 256])
    return bytes(result)


def padded(password: str) -> bytes:
    padding = bytes.fromhex("28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A")
    raw = password.encode("latin1")
    return (raw + padding)[:32]


def encrypted() -> bytes:
    password = "fixture-password"
    permissions = -44
    owner = rc4(hashlib.md5(padded(password)).digest()[:5], padded(password))
    digest = hashlib.md5()
    digest.update(padded(password))
    digest.update(owner)
    digest.update(struct.pack("<i", permissions))
    digest.update(FIXED_ID)
    key = digest.digest()[:5]
    user = rc4(key, bytes.fromhex("28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A"))
    plaintext = b"BT (SANITIZATION_ENCRYPTED_PAYLOAD_001) Tj ET"
    object_key = hashlib.md5(key + struct.pack("<I", 4)[:3] + b"\x00\x00").digest()[:10]
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R >>\nendobj\n",
        stream(4, rc4(object_key, plaintext)),
        f"5 0 obj\n<< /Filter /Standard /V 1 /R 2 /Length 40 /P {permissions} /O <{owner.hex().upper()}> /U <{user.hex().upper()}> >>\nendobj\n".encode("latin1"),
    ]
    identifier = FIXED_ID.hex().upper()
    result = classic_pdf(objects, f"/Encrypt 5 0 R /ID [<{identifier}> <{identifier}>]")
    assert b"SANITIZATION_ENCRYPTED_PAYLOAD_001" not in result
    return result


def signature() -> bytes:
    objects = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [5 0 R] >> /Perms << /DocMDP 6 0 R >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /Annots [5 0 R] >>\nendobj\n",
        stream(4, b"q Q"),
        b"5 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Sig /T (SignatureFixture) /V 6 0 R /Rect [0 0 0 0] >>\nendobj\n",
        b"6 0 obj\n<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 100 200 300] /Contents <00000000> /Reference [<< /TransformMethod /DocMDP /TransformParams << /P 2 /V /1.2 >> >> << /TransformMethod /FieldMDP /TransformParams << /Action /All /V /1.2 >> >>] >>\nendobj\n",
    ]
    return classic_pdf(objects)


def dangling_critical_references() -> bytes:
    return page_base(
        catalog_extra="/OpenAction 99 0 R /Names << /EmbeddedFiles 98 0 R >>",
    )


FIXTURES = {
    "clean-minimal.pdf": clean,
    "metadata-paths.pdf": metadata,
    "javascript-actions.pdf": javascript_actions,
    "combined-names.pdf": combined_names,
    "openaction-destination.pdf": openaction_destination,
    "action-chain-network.pdf": action_chain_network,
    "embedded-files-af.pdf": embedded_files,
    "xfa-single-stream.pdf": xfa_single_stream,
    "unsupported-active-content.pdf": unsupported_active_content,
    "incremental-revision.pdf": incremental_revision,
    "object-xref-stream.pdf": object_xref_stream,
    "incremental-xref-stream-chain.pdf": incremental_xref_stream_chain,
    "hybrid-xref.pdf": hybrid_xref,
    "encrypted.pdf": encrypted,
    "signature.pdf": signature,
    "dangling-critical-references.pdf": dangling_critical_references,
}


MANIFEST_ROWS = [
    ("clean-minimal.pdf", "Clean parser and PDFium reopen baseline", [], [], [], "", "none", "none", "preserve", "reopen and preserve invariants", "No active-content findings."),
    ("metadata-paths.pdf", "Info, XMP, page/object metadata, private data and ID", ["document_info", "xmp_metadata", "object_metadata", "document_identifier", "private_application_data"], ["info", "catalog_metadata", "page_metadata", "xobject_metadata", "piece_info", "document_id"], ["document_info", "xmp_metadata", "object_metadata", "document_identifier"], "", "none", "metadata unavailable", "remove selected metadata paths", "forbidden paths and fingerprints absent", "Fixed dates and ID."),
    ("javascript-actions.pdf", "Document, page, annotation and form JavaScript representations", ["document_javascript", "additional_actions", "form_actions"], ["names_javascript", "open_action_javascript", "catalog_aa", "page_aa", "annotation_action", "widget_action", "literal_hex_stream_payloads"], ["document_javascript", "additional_actions"], "", "none", "scripts and form automation stop", "remove targeted actions while retaining Dests and fields", "all action paths independently rescanned", "Covers literal, hex and stream JS payloads."),
    ("combined-names.pdf", "Dests, JavaScript and EmbeddedFiles sibling branches in one Names dictionary", ["document_javascript", "embedded_files"], ["names_dests", "names_javascript", "embedded_files_name_tree", "sibling_preservation"], ["document_javascript", "embedded_files"], "", "none", "scripts and attachments stop independently", "remove only the selected Names branch and prune detached payload", "Dests and non-target siblings match the selected control", "Executable selective-mutation evidence; the whole Names dictionary is retained."),
    ("openaction-destination.pdf", "Valid destination-shaped OpenAction negative control", [], ["open_action_destination_not_action"], [], "", "none", "none", "preserve", "destination remains and no JavaScript finding", "OpenAction is not always an action."),
    ("action-chain-network.pdf", "Nested and cyclic Launch/network/action Next graph", ["launch_actions", "uri_actions", "remote_navigation_actions", "form_submission_actions"], ["launch", "uri", "goto_remote", "goto_embedded", "submit_form", "import_data", "next_array", "cycle"], ["launch_actions", "uri_actions", "remote_navigation_actions", "form_submission_actions"], "", "none", "external navigation and submission stop", "remove action edges and retain link when safe", "bounded cycle-safe rescan", "Local GoTo is intentionally absent."),
    ("embedded-files-af.pdf", "EmbeddedFiles name tree, EF/RF, Catalog/Page/Annotation/Structure AF and FileAttachment", ["embedded_files", "associated_files", "file_attachment_annotations", "portfolio"], ["embedded_files_name_tree", "filespec", "ef", "rf", "embedded_file_stream", "catalog_af", "page_af", "annotation_af", "structure_element_af", "direct_filespec", "indirect_filespec", "file_attachment", "collection"], ["embedded_files", "associated_files", "file_attachment_annotations"], "", "none", "attachments and portfolio behavior removed", "detach every owner edge then prune payload", "paths, duplicate object IDs and embedded payload fingerprints absent", "The same indirect FileSpec is shared across name-tree and four AF owners; Structure AF also has a direct FileSpec."),
    ("xfa-single-stream.pdf", "XFA direct single indirect-stream reference representation", ["xfa"], ["xfa_single_reference", "xfa_stream"], [], "xfa", "possible visual and form loss", "dynamic form stops", "block; controlled edge removal is characterization only", "blocking finding remains explicit", "Single-stream reachability and pruning do not imply generic XFA support."),
    ("unsupported-active-content.pdf", "XFA array, optional content OCMD/annotation and multimedia blocker corpus", ["xfa", "optional_content", "rich_media", "rendition", "multimedia"], ["xfa_array", "xfa_streams", "ocproperties", "ocg", "ocmd", "marked_content", "annotation_oc", "rich_media", "screen", "movie", "sound", "3d", "rendition"], [], "xfa, optional_content, rich_media", "possible visual and form loss", "dynamic forms, layers and multimedia may stop", "block; do not transform", "blocking findings remain explicit", "Removing OCProperties leaves marked content, OCMD and annotation OC reachable."),
    ("incremental-revision.pdf", "Incremental history with old JavaScript and new metadata", ["incremental_revisions", "document_javascript", "xmp_metadata"], ["prev", "multiple_xref", "old_revision_javascript", "current_metadata"], ["document_javascript", "xmp_metadata"], "", "none", "scripts and metadata removed", "remove references, prune, clear Prev and full-save", "single rewrite with old marker absent", "Two deterministic revisions."),
    ("object-xref-stream.pdf", "Object stream and xref-stream parser capability", ["object_stream", "xref_stream", "object_metadata"], ["compressed_object", "xref_stream"], ["object_metadata"], "", "none", "metadata unavailable", "full rewrite may expand compressed objects", "reopen and fingerprint scan", "PDF 1.5."),
    ("incremental-xref-stream-chain.pdf", "Two real incremental xref-stream revisions linked by Prev", ["xref_stream", "incremental_revisions", "xmp_metadata"], ["xref_stream_chain", "prev", "latest_object_resolution", "physical_old_revision"], ["xmp_metadata"], "", "none", "metadata unavailable", "remove selected edges, prune, normalize trailer and full-save", "single rewrite, output-local xref and old marker absence", "Revision two xref stream Prev points to revision one xref stream."),
    ("hybrid-xref.pdf", "Hybrid-reference and incremental parser capability", ["hybrid_xref", "incremental_revisions", "object_metadata"], ["xref_table", "xref_stream", "xrefstm", "compressed_object", "prev"], ["object_metadata"], "", "none", "metadata unavailable", "full rewrite to one canonical xref form", "reopen with no Prev/XRefStm history", "Hybrid fixture deliberately combines table and stream."),
    ("encrypted.pdf", "Credential and encryption rewrite characterization", ["encryption"], ["standard_security_handler", "credential_required"], [], "encrypted_input", "none", "decrypted rewrite changes security semantics", "block production execution", "no publication", "Password is synthetic: fixture-password."),
    ("signature.pdf", "Signature, ByteRange, DocMDP and FieldMDP inspection", ["digital_signatures"], ["signature_field", "signature_dictionary", "byte_range", "doc_mdp", "field_mdp"], [], "signed_input_acknowledgement", "none", "existing signatures invalidated by rewrite", "require explicit acknowledgement in future", "signature facts independently rescanned", "Cryptographic validity is not claimed."),
    ("dangling-critical-references.pdf", "Reachable dangling action and EmbeddedFiles references", ["malformed_graph", "document_javascript", "embedded_files"], ["unresolved_open_action", "unresolved_embedded_files"], [], "unresolved_critical_references", "unknown", "unknown because graph is incomplete", "block before mutation", "every critical reference must resolve", "Parser-open success is not graph-completeness evidence."),
]


def build_manifest(output: Path, generated: dict[str, bytes]) -> dict:
    fixtures = []
    for row in MANIFEST_ROWS:
        name, purpose, categories, inspection, controls, blockers, visual, functional, transform, verify, notes = row
        payload = generated[name]
        fixtures.append({
            "name": name,
            "purpose": purpose,
            "threatCategories": categories,
            "expectedInspection": inspection,
            "candidateControls": controls,
            "blockingUnsupported": [item.strip() for item in blockers.split(",") if item.strip()],
            "expectedVisualImpact": visual,
            "expectedFunctionalImpact": functional,
            "futureTransformExpectation": transform,
            "futureVerificationExpectation": verify,
            "notes": notes,
            "sha256": hashlib.sha256(payload).hexdigest(),
            "byteSize": len(payload),
        })
    return {
        "schemaVersion": "sanitization-fixture-manifest/1",
        "description": "Deterministic synthetic 8C.1 sanitization characterization corpus; never user data.",
        "fixtures": fixtures,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, default=ROOT)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    generated = {name: builder() for name, builder in FIXTURES.items()}
    for name, payload in generated.items():
        (args.output / name).write_bytes(payload)
    manifest = build_manifest(args.output, generated)
    (args.output / "manifest.json").write_text(
        json.dumps(manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
