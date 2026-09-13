"""The verification loop: .pptx -> .pdf -> PNGs -> contact sheets.

This module is the reason the skill exists. A deck generator without a render step
produces plausible-looking garbage, and the defects it produces are invisible in
the source: a drop shadow, a centred bullet, a title overlapping its own rule, a
caption sitting on top of a panel. Every one of those was found here and none of
them by reading code.

Two artifacts come out, because they answer different questions:

  render/slide-NN.png       full resolution, for reading a single slide closely
  render/contact-sheet-K.png  a 3x3 grid, for judging layout across the whole deck
                            in one look — balance, rhythm, where the eye snags

The contact sheet is the one that matters. A deck is a sequence, and a sequence
cannot be judged one slide at a time.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

from .errors import DeckError

PER_SHEET = 9
COLUMNS = 3


def render(pptx: str | Path, *, out_dir: str | Path | None = None,
           pdf_dir: str | Path | None = None, dpi: int = 110,
           sheets: bool = True) -> dict:
    """Convert a deck to PDF and then to PNGs plus contact sheets.

    `out_dir` holds the working files: the PDF, one PNG per slide and the contact
    sheets. `pdf_dir` is where the PDF itself is wanted, normally the deck's own
    directory beside the `.pptx`, so the deliverable folder holds both. The two may be
    the same directory, and then no copy is made.

    Returns `{"pdf": Path, "exported": Path | None, "slides": [Path], "sheets": [Path]}`.
    """
    pptx = Path(pptx)
    if not pptx.exists():
        raise DeckError(
            f"cannot render {pptx}: not built yet.\n"
            f"  Run `scripts/deck build <deck.py>` first."
        )
    out_dir = Path(out_dir) if out_dir else pptx.parent / "render"
    out_dir.mkdir(parents=True, exist_ok=True)

    pdf = _to_pdf(pptx, out_dir)
    slides = _to_pngs(pdf, out_dir, dpi)
    contact_sheets = _contact_sheets(slides, out_dir) if sheets else []
    exported = _export(pdf, pdf_dir)
    return {"pdf": pdf, "exported": exported, "slides": slides,
            "sheets": contact_sheets}


def _export(pdf: Path, pdf_dir: str | Path | None) -> Path | None:
    """Copy the PDF where it is wanted, when that is somewhere else.

    The recurring request is the deck as a PDF in the same folder as the `.pptx`, so
    that folder is the deliverable. Copying costs nothing and stays idempotent.
    """
    if pdf_dir is None:
        return None
    target = Path(pdf_dir) / pdf.name
    if target.resolve() == pdf.resolve():
        return None
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(pdf, target)
    return target


def _to_pdf(pptx: Path, out_dir: Path) -> Path:
    soffice = shutil.which("soffice") or shutil.which("libreoffice")
    if not soffice:
        raise DeckError(
            "LibreOffice not found on PATH, so the deck cannot be rendered.\n"
            "  macOS:  brew install --cask libreoffice\n"
            "  Linux:  apt-get install -y libreoffice\n"
            "  Rendering is not optional: without it the deck is unverified."
        )
    done = subprocess.run(
        [soffice, "--headless", "--convert-to", "pdf", pptx.name, "--outdir", "."],
        cwd=pptx.parent, capture_output=True, text=True, timeout=600,
    )
    pdf = pptx.parent / f"{pptx.stem}.pdf"
    if not pdf.exists():
        raise DeckError(
            f"LibreOffice did not produce a PDF for {pptx.name}.\n"
            f"  stdout: {done.stdout.strip()[:400]}\n"
            f"  stderr: {done.stderr.strip()[:400]}"
        )
    target = out_dir / pdf.name
    if target.resolve() != pdf.resolve():
        shutil.move(str(pdf), str(target))
    return target


def export_pdf(pptx: str | Path, pdf_dir: str | Path | None = None) -> Path:
    """Convert the deck to PDF and leave it beside the `.pptx`.

    `build` calls this when `Deck(export_pdf=True)`, so the deck folder holds both the
    `.pptx` and the PDF and the export is not a step to remember. It runs LibreOffice
    but not pymupdf, which is why `build` keeps its lighter dependency set.
    """
    pptx = Path(pptx)
    if not pptx.exists():
        raise DeckError(
            f"cannot export {pptx}: not built yet.\n"
            f"  Run `scripts/deck build <deck.py>` first."
        )
    return _to_pdf(pptx, Path(pdf_dir) if pdf_dir is not None else pptx.parent)


def _to_pngs(pdf: Path, out_dir: Path, dpi: int) -> list[Path]:
    try:
        import pymupdf
    except ImportError as exc:  # pragma: no cover - launcher always provides it
        raise DeckError(
            "pymupdf is required to rasterise the PDF for visual inspection.\n"
            "  The launcher installs it on demand: `scripts/deck render <deck.py>`"
        ) from exc

    document = pymupdf.open(str(pdf))
    pages = []
    for index in range(document.page_count):
        target = out_dir / f"slide-{index + 1:02d}.png"
        document[index].get_pixmap(dpi=dpi).save(target)
        pages.append(target)
    document.close()
    if not pages:
        raise DeckError(f"{pdf} rendered as an empty document.")
    return pages


def _contact_sheets(slides: list[Path], out_dir: Path) -> list[Path]:
    from PIL import Image

    with Image.open(slides[0]) as first:
        tile_w, tile_h = first.size
    gap = max(int(tile_w * 0.012), 8)
    rows = PER_SHEET // COLUMNS
    sheet_w = COLUMNS * tile_w + (COLUMNS + 1) * gap
    sheet_h = rows * tile_h + (rows + 1) * gap

    made = []
    for sheet_index in range(0, len(slides), PER_SHEET):
        chunk = slides[sheet_index:sheet_index + PER_SHEET]
        sheet = Image.new("RGB", (sheet_w, sheet_h), (235, 235, 232))
        for offset, slide in enumerate(chunk):
            row, column = divmod(offset, COLUMNS)
            with Image.open(slide) as tile:
                x = gap + column * (tile_w + gap)
                y = gap + row * (tile_h + gap)
                sheet.paste(tile, (x, y))
        target = out_dir / f"contact-sheet-{sheet_index // PER_SHEET + 1}.png"
        sheet.save(target)
        made.append(target)
    return made


def describe(result: dict) -> str:
    lines = [f"pdf     {result['pdf']}"]
    if result.get("exported"):
        lines.append(f"        also {result['exported']}")
    lines.append(f"slides  {len(result['slides'])} PNGs in {result['slides'][0].parent}")
    for sheet in result["sheets"]:
        lines.append(f"sheet   {sheet}")
    return "\n".join(lines)
