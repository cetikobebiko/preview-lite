"""Generate deterministic Prompt 4 reorder fixtures.

Requires the repository tooling environment's reportlab and pypdf packages.
The generated files are committed; tests never generate or download fixtures.
"""

from io import BytesIO
from pathlib import Path

from pypdf import PdfReader, PdfWriter
from pypdf.annotations import Text
from reportlab.pdfgen import canvas


ROOT = Path(__file__).resolve().parent


def reportlab_pages(*, with_form: bool = False) -> bytes:
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


def safe_writer() -> PdfWriter:
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


def write(writer: PdfWriter, name: str) -> None:
    with (ROOT / name).open("wb") as output:
        writer.write(output)


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
