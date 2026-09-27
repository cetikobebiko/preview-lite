"""Generate deterministic Prompt 4 reorder fixtures.

Requires the repository tooling environment's reportlab and pypdf packages.
The generated files are committed; tests never generate or download fixtures.
"""

from __future__ import annotations

import argparse
from io import BytesIO
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parent


def check_dependencies() -> bool:
    """Non-destructively check if reportlab and pypdf are available."""
    missing = []
    try:
        import reportlab  # noqa: F401
    except ImportError:
        missing.append("reportlab")

    try:
        import pypdf  # noqa: F401
    except ImportError:
        missing.append("pypdf")

    if missing:
        print(f"TOOL_UNAVAILABLE_OFFLINE: Missing local dependencies: {', '.join(missing)}")
        return False
    print("Dependencies 'reportlab' and 'pypdf' are available locally.")
    return True


def reportlab_pages(*, with_form: bool = False) -> bytes:
    from reportlab.pdfgen import canvas

    stream = BytesIO()
    pdf = canvas.Canvas(stream, pagesize=(300, 400), invariant=1)
    sizes = [(300, 400), (400, 500), (500, 600)]
    labels = ["PAGE-ONE", "PAGE-TWO", "PAGE-THREE"]
    for index, ((width, height), label) in enumerate(zip(sizes, labels)):
        pdf.setPageSize((width, height))
        pdf.setFont("Helvetica", 18)
        pdf.drawString(40, height - 60, label)
        if with_form and index == 0:
            pdf.acroForm.textfield(
                name="fixture-field",
                value="fixture-value",
                x=40,
                y=250,
                width=140,
                height=24,
            )
        pdf.showPage()
    pdf.save()
    return stream.getvalue()


def safe_writer():
    from pypdf import PdfReader, PdfWriter
    from pypdf.annotations import Text

    source = PdfReader(BytesIO(reportlab_pages()))
    writer = PdfWriter()
    for page in source.pages:
        writer.add_page(page)
    writer.pages[1].rotate(90)
    writer.add_annotation(
        page_number=2,
        annotation=Text(
            rect=(40, 450, 70, 480),
            text="Reorder fixture annotation",
            open=False,
        ),
    )
    writer.metadata = None
    return writer


def generate_reorder_fixtures(target_dir: Path) -> dict[str, int]:
    """Generates reorder PDF fixtures into the specified target directory."""
    from pypdf import PdfReader, PdfWriter

    target_dir.mkdir(parents=True, exist_ok=True)
    results = {}

    def write(writer, name: str) -> None:
        out_path = target_dir / name
        with out_path.open("wb") as output:
            writer.write(output)
        results[name] = out_path.stat().st_size
        print(f"Wrote {name}: {results[name]} bytes")

    write(safe_writer(), "reorder-rich.pdf")

    metadata = safe_writer()
    metadata.add_metadata({"/Title": "Must be rejected"})
    write(metadata, "reorder-with-metadata.pdf")

    outline = safe_writer()
    outline.add_outline_item("Must be rejected", 0)
    write(outline, "reorder-with-outline.pdf")

    named_destination = safe_writer()
    named_destination.add_named_destination("must-be-rejected", 0)
    write(named_destination, "reorder-with-named-destination.pdf")

    form_source = PdfReader(BytesIO(reportlab_pages(with_form=True)))
    form = PdfWriter()
    form.clone_document_from_reader(form_source)
    form.metadata = None
    write(form, "reorder-with-form.pdf")

    return results


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Deterministic Reorder Fixture Generator (toolchain_unpinned)"
    )
    parser.add_argument(
        "--check-dependencies",
        action="store_true",
        help="Non-destructively probe local dependency availability",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=None,
        help="Explicit output directory for fixtures (mandatory for generation)",
    )
    args = parser.parse_args()

    if args.check_dependencies:
        ok = check_dependencies()
        sys.exit(0 if ok else 1)
    elif args.output is not None:
        if not check_dependencies():
            sys.exit(1)
        generate_reorder_fixtures(args.output)
    else:
        parser.print_help()
        sys.exit(0)


if __name__ == "__main__":
    main()

