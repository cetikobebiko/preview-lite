#!/usr/bin/env python3
"""
Deterministic PDF Redaction Fixture Generator & Manifest Builder.
Uses pure Python standard library to generate valid PDF 1.4 / 1.5 test fixtures.
Automatically calculates exact byte /Length for all PDF streams (text, binary, ObjStm, XRef).
Produces canonical manifest.json with deterministic SHA-256 and byte sizes.
No external pip dependencies required.
"""

import os
import shutil
import zlib
import json
import hashlib
import struct
from pathlib import Path

ROOT = Path(__file__).resolve().parent

def build_stream_object_bytes(obj_id: int, dict_entries: str, stream_bytes: bytes) -> bytes:
    """
    Constructs a PDF stream object byte array with exact stream /Length calculated automatically.
    Length equals exact byte count between 'stream\n' and '\nendstream'.
    """
    length = len(stream_bytes)
    dict_str = f"<< /Length {length} {dict_entries} >>" if dict_entries.strip() else f"<< /Length {length} >>"
    header = f"{obj_id} 0 obj\n{dict_str}\nstream\n".encode('latin1')
    footer = b"\nendstream\nendobj\n"
    return header + stream_bytes + footer

def build_pdf_bytes_from_raw_objects(raw_objects: list[bytes], trailer_extras: str = "") -> bytes:
    """Constructs a valid PDF 1.4 from raw byte objects with exact cross-reference table."""
    buf = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    
    for obj in raw_objects:
        offsets.append(len(buf))
        buf.extend(obj)
        if not obj.endswith(b'\n'):
            buf.extend(b'\n')
            
    xref_offset = len(buf)
    buf.extend(f"xref\n0 {len(raw_objects) + 1}\n".encode('latin1'))
    buf.extend(b"0000000000 65535 f \n")
    for off in offsets[1:]:
        buf.extend(f"{off:010d} 00000 n \n".encode('latin1'))
        
    trailer = (
        f"trailer\n"
        f"<< /Size {len(raw_objects) + 1} /Root 1 0 R {trailer_extras} >>\n"
        f"startxref\n"
        f"{xref_offset}\n"
        f"%%EOF\n"
    )
    buf.extend(trailer.encode('latin1'))
    return bytes(buf)

def generate_simple_text() -> bytes:
    stream_content = b"BT /F1 12 Tf 50 750 TD (CONFIDENTIAL TC: 12345678901 EMAIL: user@example.com) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", stream_content)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_rotations() -> bytes:
    stream_content = b"BT /F1 14 Tf 50 750 TD (ROTATED PAGE) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R 6 0 R] /Count 4 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate 0 /Resources << /Font << /F1 7 0 R >> >> /Contents 8 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate 90 /Resources << /Font << /F1 7 0 R >> >> /Contents 8 0 R >>\nendobj\n",
        b"5 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate 180 /Resources << /Font << /F1 7 0 R >> >> /Contents 8 0 R >>\nendobj\n",
        b"6 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Rotate 270 /Resources << /Font << /F1 7 0 R >> >> /Contents 8 0 R >>\nendobj\n",
        b"7 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(8, "", stream_content)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_cropbox_mediabox_diff() -> bytes:
    stream_content = b"BT /F1 12 Tf 70 700 TD (CROPBOX DIFFERENT) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /CropBox [50 50 550 750] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", stream_content)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_nonzero_origin() -> bytes:
    stream_content = b"BT /F1 12 Tf 100 750 TD (NONZERO ORIGIN PAGE) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [50 50 650 850] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", stream_content)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_negative_origin() -> bytes:
    stream_content = b"BT /F1 12 Tf -50 600 TD (NEGATIVE ORIGIN PAGE) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [-100 -100 500 700] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", stream_content)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_image_only() -> bytes:
    img_data = bytes([0, 255, 255, 0])
    content_stream = b"q 400 0 0 400 100 200 cm /Im1 Do Q"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /XObject << /Im1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        build_stream_object_bytes(4, "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", img_data),
        build_stream_object_bytes(5, "", content_stream)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_text_image() -> bytes:
    img_data = bytes([128, 128, 128, 128])
    content_stream = b"BT /F1 12 Tf 50 750 TD (TEXT AND IMAGE PAGE) Tj ET q 200 0 0 200 50 500 cm /Im1 Do Q"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> /XObject << /Im1 5 0 R >> >> /Contents 6 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", img_data),
        build_stream_object_bytes(6, "", content_stream)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_annotations_fixture() -> bytes:
    content_stream = b"BT /F1 12 Tf 50 750 TD (ANNOTATED PAGE) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Annots [6 0 R] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", content_stream),
        b"6 0 obj\n<< /Type /Annot /Subtype /Text /Rect [100 700 120 720] /Contents (Sensitive Note) /C [1 1 0] >>\nendobj\n"
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_acroform_widget() -> bytes:
    content_stream = b"BT /F1 12 Tf 50 750 TD (ACROFORM FORM PAGE) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] /NeedAppearances true >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Annots [6 0 R] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", content_stream),
        b"6 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Tx /T (CustomerName) /V (Jane Doe) /Rect [50 650 250 680] >>\nendobj\n"
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_xfa_fixture() -> bytes:
    content_stream = b"BT /F1 12 Tf 50 750 TD (XFA PAGE) Tj ET"
    xfa_xml = b"<template><subform name='form1'></subform></template>"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] /XFA 6 0 R >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", content_stream),
        build_stream_object_bytes(6, "", xfa_xml)
    ]
    return build_pdf_bytes_from_raw_objects(objs)

ENCRYPTED_REDACTION_FIXTURE_TEXT = "ENCRYPTED REDACTION FIXTURE SECRET"

def rc4_crypt(key: bytes, data: bytes) -> bytes:
    S = list(range(256))
    j = 0
    for i in range(256):
        j = (j + S[i] + key[i % len(key)]) % 256
        S[i], S[j] = S[j], S[i]
    i = j = 0
    res = bytearray()
    for b in data:
        i = (i + 1) % 256
        j = (j + S[i]) % 256
        S[i], S[j] = S[j], S[i]
        res.append(b ^ S[(S[i] + S[j]) % 256])
    return bytes(res)

def _pad_password(password: str) -> bytes:
    pad = b"\x28\xbf\x4e\x5e\x4e\x75\x8a\x41\x64\x00\x4e\x56\xff\xfa\x01\x08\x2e\x2e\x00\xb6\xd0\x68\x3e\x80\x2f\x0c\xa9\xfe\x64\x53\x69\x7a"
    p = password.encode('latin1')
    if len(p) >= 32:
        return p[:32]
    return p + pad[:32 - len(p)]

def generate_encrypted_redaction_fixture() -> bytes:
    """
    Generates a single-page PDF protected with PDF Standard Security Handler Rev 2 (40-bit RC4).
    Password: 'fixture-password'.
    Page contents stream is encrypted with RC4 and contains ENCRYPTED_REDACTION_FIXTURE_TEXT.
    Raw PDF bytes do NOT contain the plaintext marker.
    All random values (ID, salt) are fixed for determinism.
    """
    user_pass = "fixture-password"
    owner_pass = "fixture-password"
    p_val = -44
    id_bytes = b"FIXED_FILE_ID_16"

    user_padded = _pad_password(user_pass)
    owner_padded = _pad_password(owner_pass)

    owner_hash = hashlib.md5(owner_padded).digest()
    o_key = owner_hash[:5]
    o_bytes = rc4_crypt(o_key, user_padded)

    m = hashlib.md5()
    m.update(user_padded)
    m.update(o_bytes)
    m.update(struct.pack("<i", p_val))
    m.update(id_bytes)
    enc_key = m.digest()[:5]

    pad_const = b"\x28\xbf\x4e\x5e\x4e\x75\x8a\x41\x64\x00\x4e\x56\xff\xfa\x01\x08\x2e\x2e\x00\xb6\xd0\x68\x3e\x80\x2f\x0c\xa9\xfe\x64\x53\x69\x7a"
    u_bytes = rc4_crypt(enc_key, pad_const)

    content_plaintext = f"BT /F1 12 Tf 50 750 TD ({ENCRYPTED_REDACTION_FIXTURE_TEXT}) Tj ET".encode('latin1')

    key_buf = enc_key + struct.pack("<I", 5)[:3] + struct.pack("<H", 0)
    obj5_key = hashlib.md5(key_buf).digest()[:min(len(enc_key) + 5, 16)]
    content_encrypted = rc4_crypt(obj5_key, content_plaintext)

    def hex_str(b: bytes) -> str:
        return "<" + b.hex().upper() + ">"

    enc_dict_str = (
        f"<< /Filter /Standard /V 1 /R 2 /Length 40 /P {p_val} "
        f"/O {hex_str(o_bytes)} /U {hex_str(u_bytes)} >>"
    )

    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", content_encrypted),
        f"6 0 obj\n{enc_dict_str}\nendobj\n".encode('latin1')
    ]

    trailer_extras = f"/Encrypt 6 0 R /ID [{hex_str(id_bytes)} {hex_str(id_bytes)}]"
    raw_pdf = build_pdf_bytes_from_raw_objects(objs, trailer_extras=trailer_extras)

    assert ENCRYPTED_REDACTION_FIXTURE_TEXT.encode('latin1') not in raw_pdf, \
        "Plaintext marker must not be visible in raw encrypted PDF bytes"

    return raw_pdf

def generate_signature_marked_byterange() -> bytes:
    content_stream = b"BT /F1 12 Tf 50 750 TD (SIGNED DOCUMENT) Tj ET"
    objs = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [6 0 R] >> >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Annots [6 0 R] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", content_stream),
        b"6 0 obj\n<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /V 7 0 R /Rect [0 0 0 0] >>\nendobj\n",
        b"7 0 obj\n<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached /ByteRange [0 100 200 300] /Contents <00000000> >>\nendobj\n"
    ]
    return build_pdf_bytes_from_raw_objects(objs)

def generate_incremental_revision() -> bytes:
    rev1_stream = b"BT /F1 12 Tf 50 750 TD (REVISION_ONE_REACHABLE_MARKER) Tj ET"
    objs_rev1 = [
        b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
        b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
        b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [6 0 R 7 0 R] >>\nendobj\n",
        b"4 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n",
        build_stream_object_bytes(5, "", rev1_stream),
        b"6 0 obj\n<< /Type /Annot /Subtype /Text /Contents (REVISION_ONE_REACHABLE_MARKER) /Rect [5 5 15 15] >>\nendobj\n",
        b"7 0 obj\n<< /Type /Annot /Subtype /Text /Contents (UNREACHABLE_OLD_OBJECT_MARKER) /Rect [10 10 30 30] >>\nendobj\n"
    ]
    rev1_bytes = build_pdf_bytes_from_raw_objects(objs_rev1)
    
    prev_startxref = rev1_bytes.rfind(b"startxref")
    lines = rev1_bytes[prev_startxref:].splitlines()
    prev_xref_val = lines[1].decode('latin1').strip()

    # Revision 2 adds object 8 (Annot with REVISION_TWO_REACHABLE_MARKER) and updates Page 3 to reference [6 0 R, 8 0 R], dropping 7 0 R
    obj8_bytes = b"8 0 obj\n<< /Type /Annot /Subtype /Text /Contents (REVISION_TWO_REACHABLE_MARKER) /Rect [20 20 40 40] >>\nendobj\n"
    obj3_updated_bytes = b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [6 0 R 8 0 R] >>\nendobj\n"
    
    obj8_offset = len(rev1_bytes)
    obj3_updated_offset = obj8_offset + len(obj8_bytes)
    
    xref_offset = obj3_updated_offset + len(obj3_updated_bytes)
    
    xref_table = f"xref\n3 1\n{obj3_updated_offset:010d} 00000 n \n8 1\n{obj8_offset:010d} 00000 n \n".encode('latin1')
    trailer = f"trailer\n<< /Size 9 /Root 1 0 R /Prev {prev_xref_val} >>\nstartxref\n{xref_offset}\n%%EOF\n".encode('latin1')
    
    return rev1_bytes + obj8_bytes + obj3_updated_bytes + xref_table + trailer

def generate_object_stream() -> bytes:
    obj5_dict = "<< /Type /Metadata /Marker (COMPRESSED_OBJ_5_MARKER) >>"
    obj6_dict = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
    
    offsets_header = f"5 0 6 {len(obj5_dict) + 1}\n"
    obj_stm_content_bytes = (offsets_header + obj5_dict + "\n" + obj6_dict).encode('latin1')
    
    buf = bytearray(b"%PDF-1.5\n%\xe2\xe3\xcf\xd3\n")
    
    off1 = len(buf)
    buf.extend(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
    
    off2 = len(buf)
    buf.extend(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n")
    
    off3 = len(buf)
    buf.extend(b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /Font << /F1 6 0 R >> >> /Contents 7 0 R >>\nendobj\n")
    
    off4 = len(buf)
    obj4_bytes = build_stream_object_bytes(4, f"/Type /ObjStm /N 2 /First {len(offsets_header)}", obj_stm_content_bytes)
    buf.extend(obj4_bytes)
    
    off7 = len(buf)
    obj7_stream = b"BT /F1 12 Tf 50 750 TD (OBJSTM PAGE) Tj ET"
    obj7_bytes = build_stream_object_bytes(7, "", obj7_stream)
    buf.extend(obj7_bytes)

    off8 = len(buf)
    entries = bytearray()
    entries.extend(bytes([0, 0, 0, 0])) # 0
    entries.extend(bytes([1, (off1 >> 8) & 0xff, off1 & 0xff, 0])) # 1
    entries.extend(bytes([1, (off2 >> 8) & 0xff, off2 & 0xff, 0])) # 2
    entries.extend(bytes([1, (off3 >> 8) & 0xff, off3 & 0xff, 0])) # 3
    entries.extend(bytes([1, (off4 >> 8) & 0xff, off4 & 0xff, 0])) # 4
    entries.extend(bytes([2, 0, 4, 0])) # 5 (in ObjStm 4 index 0)
    entries.extend(bytes([2, 0, 4, 1])) # 6 (in ObjStm 4 index 1)
    entries.extend(bytes([1, (off7 >> 8) & 0xff, off7 & 0xff, 0])) # 7
    entries.extend(bytes([1, (off8 >> 8) & 0xff, off8 & 0xff, 0])) # 8
    
    obj8_bytes = build_stream_object_bytes(8, f"/Type /XRef /Size 9 /W [1 2 1] /Root 1 0 R", bytes(entries))
    buf.extend(obj8_bytes)
    buf.extend(f"startxref\n{off8}\n%%EOF\n".encode('latin1'))
    
    return bytes(buf)

FIXTURE_DEFINITIONS = [
    {
        "id": "simple-text",
        "filename": "simple-text.pdf",
        "purpose": "Basit metin iceren 1 sayfalik PDF 1.4 belge",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["has_text_objects", "standard_origin"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "pdfium_open_succeeds",
            "page_count_equals_1",
            "extracted_text_contains_confidential_marker",
            "has_text_page_object"
        ],
        "generator": generate_simple_text,
    },
    {
        "id": "rotations",
        "filename": "rotations.pdf",
        "purpose": "Farkli sayfa acilarina (0, 90, 180, 270 derece) sahip 4 sayfalik PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 4,
        "expected_properties": ["rotations_0_90_180_270"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "pdfium_open_succeeds",
            "page_count_equals_4",
            "page_rotations_match_0_90_180_270",
            "render_smoke_test_succeeds_all_pages"
        ],
        "generator": generate_rotations,
    },
    {
        "id": "cropbox-mediabox-diff",
        "filename": "cropbox-mediabox-diff.pdf",
        "purpose": "CropBox ve MediaBox sinirlari birbirinden farkli olan PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["cropbox_mediabox_mismatch"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "mediabox_not_equal_cropbox",
            "cropbox_left_equals_50",
            "effective_dimensions_derived_correctly"
        ],
        "generator": generate_cropbox_mediabox_diff,
    },
    {
        "id": "nonzero-origin",
        "filename": "nonzero-origin.pdf",
        "purpose": "MediaBox orijin noktasi (0,0) haricinde (50, 50) olan PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["nonzero_origin_50_50"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "mediabox_origin_left_50_bottom_50",
            "page_dimensions_correctly_offset"
        ],
        "generator": generate_nonzero_origin,
    },
    {
        "id": "negative-origin",
        "filename": "negative-origin.pdf",
        "purpose": "MediaBox orijin noktasi negatif (-100, -100) olan PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["negative_origin_neg100_neg100"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "mediabox_origin_left_neg100_bottom_neg100",
            "lopdf_pdfium_geometry_match"
        ],
        "generator": generate_negative_origin,
    },
    {
        "id": "image-only",
        "filename": "image-only.pdf",
        "purpose": "Sayfada hic metin bulunmayan, yalnizca resim nesnesi barindiran PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["image_object_only", "zero_text_objects"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "image_object_count_greater_than_0",
            "text_object_count_equals_0",
            "extracted_text_is_empty",
            "render_smoke_test_succeeds"
        ],
        "generator": generate_image_only,
    },
    {
        "id": "text-image",
        "filename": "text-image.pdf",
        "purpose": "Ayni sayfada hem metin hem resim nesneleri iceren karma PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["text_and_image_objects"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "has_text_object",
            "has_image_object",
            "extracted_text_contains_marker",
            "render_smoke_test_succeeds"
        ],
        "generator": generate_text_image,
    },
    {
        "id": "annotations-fixture",
        "filename": "annotations-fixture.pdf",
        "purpose": "Sayfada /Annot sözlüğü ve açıklamalar barındıran PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["has_annotations"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "annotation_count_greater_than_0",
            "annotation_subtype_equals_text",
            "annotation_associated_with_page_0"
        ],
        "generator": generate_annotations_fixture,
    },
    {
        "id": "acroform-widget",
        "filename": "acroform-widget.pdf",
        "purpose": "AcroForm form alanları ve Widget açıklamaları içeren PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["has_acroform", "has_widgets"],
        "expected_planning_outcome": "warning",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "catalog_has_acroform",
            "fields_array_non_empty",
            "field_type_equals_tx",
            "widget_annotation_present",
            "field_and_widget_relationship_resolved"
        ],
        "generator": generate_acroform_widget,
    },
    {
        "id": "xfa-fixture",
        "filename": "xfa-fixture.pdf",
        "purpose": "AcroForm içinde dinamik /XFA stream barındıran PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["has_xfa_form"],
        "expected_planning_outcome": "blocked",
        "expected_blocker": "REDACTION_XFA_INPUT_UNSUPPORTED",
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "blocked_unsupported",
        "semantic_assertions": [
            "catalog_has_acroform",
            "acroform_has_xfa_entry",
            "planner_detects_xfa_blocker",
            "distinct_from_standard_acroform"
        ],
        "generator": generate_xfa_fixture,
    },
    {
        "id": "encrypted-redaction",
        "filename": "encrypted-redaction.pdf",
        "purpose": "Gerçek parola korumalı ve içerik seviyesinde şifreli PDF belge",
        "encrypted": True,
        "test_credential": "fixture-password",
        "expected_page_count": 1,
        "expected_text": ENCRYPTED_REDACTION_FIXTURE_TEXT,
        "expected_properties": ["encrypted_standard_security_handler"],
        "expected_planning_outcome": "blocked",
        "expected_blocker": "REDACTION_ENCRYPTED_INPUT_UNSUPPORTED",
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "blocked_unsupported",
        "semantic_assertions": [
            "open_without_password_fails",
            "open_with_wrong_password_fails",
            "open_with_correct_password_succeeds",
            "correct_password_extracts_expected_text",
            "plaintext_marker_not_present_in_raw_pdf",
            "security_handler_not_unprotected",
            "encrypted_stream_length_is_valid"
        ],
        "generator": generate_encrypted_redaction_fixture,
    },
    {
        "id": "signature-marked-byterange",
        "filename": "signature-marked-byterange.pdf",
        "purpose": "Imza işaretleyici sözlüğü ve /ByteRange barındıran PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["has_signature_dict", "has_byterange"],
        "expected_planning_outcome": "blocked",
        "expected_blocker": "REDACTION_SIGNED_INPUT_UNSUPPORTED",
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "blocked_unsupported",
        "semantic_assertions": [
            "signature_dict_present",
            "byterange_array_present",
            "planner_detects_signature_blocker",
            "no_cryptographic_validity_claimed"
        ],
        "generator": generate_signature_marked_byterange,
    },
    {
        "id": "incremental-revision",
        "filename": "incremental-revision.pdf",
        "purpose": "Birden fazla xref ve /Prev revizyon zinciri iceren PDF",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["incremental_updates", "has_prev_pointer"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "multiple_xref_sections_present",
            "trailer_has_prev_pointer",
            "contains_revision_one_reachable_marker",
            "contains_revision_two_reachable_marker",
            "contains_unreachable_old_object_marker",
            "unreachable_object_pruning_testable"
        ],
        "generator": generate_incremental_revision,
    },
    {
        "id": "object-stream",
        "filename": "object-stream.pdf",
        "purpose": "Gercek Object stream (/ObjStm) ve XRef stream kullanan PDF 1.5 belge",
        "encrypted": False,
        "test_credential": None,
        "expected_page_count": 1,
        "expected_properties": ["objstm_compressed_objects", "xref_stream"],
        "expected_planning_outcome": "ready",
        "expected_blocker": None,
        "expected_profile": "secure_rasterized_pages_v1",
        "expected_strategy": "rasterized_pages",
        "expected_verification_outcome": "verified_clean",
        "semantic_assertions": [
            "pdf_version_is_1_5",
            "objstm_dictionary_present",
            "xref_stream_present",
            "type_2_xref_entries_present",
            "compressed_object_not_top_level",
            "lopdf_resolves_compressed_object",
            "pdfium_opens_and_renders"
        ],
        "generator": generate_object_stream,
    }
]

def generate_all_fixtures_and_manifest(target_dir: Path) -> dict[str, dict]:
    target_dir.mkdir(parents=True, exist_ok=True)
    manifest_items = []

    for item in FIXTURE_DEFINITIONS:
        filename = item["filename"]
        filepath = target_dir / filename
        gen_fn = item["generator"]
        
        data = gen_fn()
        with open(filepath, "wb") as f:
            f.write(data)
            
        sha = hashlib.sha256(data).hexdigest()
        byte_size = len(data)

        manifest_item = {
            "id": item["id"],
            "filename": item["filename"],
            "purpose": item["purpose"],
            "encrypted": item["encrypted"],
            "test_credential": item["test_credential"],
            "expected_page_count": item["expected_page_count"],
            "expected_text": item.get("expected_text"),
            "expected_properties": item["expected_properties"],
            "expected_planning_outcome": item["expected_planning_outcome"],
            "expected_blocker": item["expected_blocker"],
            "expected_profile": item["expected_profile"],
            "expected_strategy": item["expected_strategy"],
            "expected_verification_outcome": item["expected_verification_outcome"],
            "sha256": sha,
            "byte_size": byte_size,
            "semantic_assertions": item["semantic_assertions"]
        }
        manifest_items.append(manifest_item)
        print(f"Generated {filename}: {byte_size} bytes, sha256: {sha}")

    manifest_data = {
        "version": 1,
        "description": "Deterministik PDF Redaksiyon Fixture Corpus ve Kanonik Manifest",
        "fixtures": manifest_items
    }

    manifest_path = target_dir / "manifest.json"
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest_data, f, indent=2, ensure_ascii=False)
        f.write("\n")
        
    print(f"Generated canonical manifest.json with {len(manifest_items)} items.")
    return manifest_data

def main():
    print("Generating deterministic redaction PDF fixtures and canonical manifest.json...")
    generate_all_fixtures_and_manifest(ROOT)
    print("All fixtures and manifest generated successfully.")

if __name__ == "__main__":
    main()
