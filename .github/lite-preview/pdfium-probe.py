#!/usr/bin/env python3
"""Load the Pdfium library an installed Lite bundle ships and render a PDF with it.

Proves, on the machine and architecture the smoke runs on, that the bundled
library loads from where the app looks for it (src-tauri/src/ocr/pdf.rs,
bundled_pdfium_candidate_paths) and actually works: it counts the pages of a
small generated two-page PDF and renders page 1, checking that the black square
drawn on it comes out black and the paper around it white.

Usage: pdfium-probe.py <path-to-libpdfium>
"""
import ctypes
import os
import platform
import sys
import tempfile


def two_page_pdf() -> bytes:
    """A minimal valid PDF: two 200x200 pages, a black 100x100 square on page 1."""
    square = b"0 0 0 rg 50 50 100 100 re f"
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 5 0 R >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
        b"<< /Length %d >>\nstream\n%s\nendstream" % (len(square), square),
    ]
    out = bytearray(b"%PDF-1.4\n")
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += b"%d 0 obj\n%s\nendobj\n" % (number, body)
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    for offset in offsets:
        out += b"%010d 00000 n \n" % offset
    out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (
        len(objects) + 1,
        xref,
    )
    return bytes(out)


def main() -> int:
    lib_path = sys.argv[1]
    print(f"python {platform.machine()} loading {lib_path}")
    pdfium = ctypes.CDLL(lib_path)

    pdfium.FPDF_InitLibrary.restype = None
    pdfium.FPDF_LoadDocument.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    pdfium.FPDF_LoadDocument.restype = ctypes.c_void_p
    pdfium.FPDF_GetLastError.restype = ctypes.c_ulong
    pdfium.FPDF_GetPageCount.argtypes = [ctypes.c_void_p]
    pdfium.FPDF_GetPageCount.restype = ctypes.c_int
    pdfium.FPDF_LoadPage.argtypes = [ctypes.c_void_p, ctypes.c_int]
    pdfium.FPDF_LoadPage.restype = ctypes.c_void_p
    pdfium.FPDFBitmap_Create.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int]
    pdfium.FPDFBitmap_Create.restype = ctypes.c_void_p
    pdfium.FPDFBitmap_FillRect.argtypes = [ctypes.c_void_p] + [ctypes.c_int] * 4 + [ctypes.c_ulong]
    pdfium.FPDFBitmap_FillRect.restype = ctypes.c_int
    pdfium.FPDF_RenderPageBitmap.argtypes = [ctypes.c_void_p, ctypes.c_void_p] + [ctypes.c_int] * 6
    pdfium.FPDF_RenderPageBitmap.restype = None
    pdfium.FPDFBitmap_GetBuffer.argtypes = [ctypes.c_void_p]
    pdfium.FPDFBitmap_GetBuffer.restype = ctypes.c_void_p
    pdfium.FPDFBitmap_GetStride.argtypes = [ctypes.c_void_p]
    pdfium.FPDFBitmap_GetStride.restype = ctypes.c_int
    for name in ("FPDFBitmap_Destroy", "FPDF_ClosePage", "FPDF_CloseDocument"):
        getattr(pdfium, name).argtypes = [ctypes.c_void_p]
        getattr(pdfium, name).restype = None

    pdfium.FPDF_InitLibrary()
    with tempfile.NamedTemporaryFile(suffix=".pdf", delete=False) as handle:
        handle.write(two_page_pdf())
        pdf_path = handle.name
    try:
        document = pdfium.FPDF_LoadDocument(pdf_path.encode(), None)
        if not document:
            print(f"FAIL: FPDF_LoadDocument error {pdfium.FPDF_GetLastError()}")
            return 1
        pages = pdfium.FPDF_GetPageCount(document)
        print(f"page count: {pages}")
        if pages != 2:
            print("FAIL: expected 2 pages")
            return 1

        page = pdfium.FPDF_LoadPage(document, 0)
        size = 200
        bitmap = pdfium.FPDFBitmap_Create(size, size, 0)
        pdfium.FPDFBitmap_FillRect(bitmap, 0, 0, size, size, 0xFFFFFFFF)
        pdfium.FPDF_RenderPageBitmap(bitmap, page, 0, 0, size, size, 0, 0)
        stride = pdfium.FPDFBitmap_GetStride(bitmap)
        buffer = ctypes.string_at(pdfium.FPDFBitmap_GetBuffer(bitmap), stride * size)

        def pixel(x: int, y: int) -> tuple:
            offset = y * stride + x * 4  # BGRx
            return tuple(buffer[offset : offset + 3])

        centre, corner = pixel(100, 100), pixel(10, 10)
        print(f"rendered page 1: centre {centre}, corner {corner}")
        pdfium.FPDFBitmap_Destroy(bitmap)
        pdfium.FPDF_ClosePage(page)
        pdfium.FPDF_CloseDocument(document)
        if max(centre) > 32 or min(corner) < 223:
            print("FAIL: page 1 did not render the black square on white")
            return 1
    finally:
        os.unlink(pdf_path)
        pdfium.FPDF_DestroyLibrary()

    print("pdfium probe OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
