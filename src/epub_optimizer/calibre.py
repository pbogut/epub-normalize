"""Select local Calibre books and update their formats through calibredb."""

import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

from epub_optimizer.core import optimize_epub, preview_epub_changes


@dataclass(frozen=True)
class Book:
    id: int
    title: str
    authors: str
    formats: dict[str, Path]
    directory: Path


def run_calibredb(arguments: list[str], library: Path | None) -> str:
    command = ["calibredb", *arguments]
    if library is not None:
        command.extend(["--with-library", str(library)])
    try:
        result = subprocess.run(command, capture_output=True, text=True, check=False)
    except FileNotFoundError as exc:
        raise RuntimeError("calibredb was not found. Install Calibre and put it on PATH.") from exc
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip() or f"exit {result.returncode}"
        raise RuntimeError(f"calibredb {arguments[0]} failed: {detail}")
    return result.stdout


def _pick(labels: list[str], prompt: str) -> int:
    if len(labels) == 1:
        return 0
    rows = [f"{index}\t{' '.join(label.split())}" for index, label in enumerate(labels)]
    # User defaults such as --multi, --filter or --expect change the output protocol.
    env = dict(os.environ)
    env.pop("FZF_DEFAULT_OPTS", None)
    env.pop("FZF_DEFAULT_OPTS_FILE", None)
    try:
        result = subprocess.run(
            ["fzf", "--no-multi", "--delimiter=\t", "--with-nth=2..", f"--prompt={prompt}"],
            input="\n".join(rows) + "\n",
            stdout=subprocess.PIPE,
            text=True,
            env=env,
            check=False,
        )
    except FileNotFoundError as exc:
        raise RuntimeError("Multiple matches found. Install fzf to choose one.") from exc
    if result.returncode in {1, 130}:
        raise KeyboardInterrupt
    if result.returncode:
        raise RuntimeError(f"fzf failed with exit code {result.returncode}.")
    selected = result.stdout.rstrip("\n")
    if selected not in rows:
        raise RuntimeError("fzf returned an invalid selection.")
    return rows.index(selected)


def _list_entries(query: str, library: str | None) -> tuple[list[dict], Path | None]:
    root = None
    if library:
        root = Path(library).expanduser().resolve()
        if not (root / "metadata.db").is_file():
            raise ValueError(f"Not a local Calibre library, metadata.db is missing: {root}")
    raw = run_calibredb(
        ["list", "--for-machine", "--fields", "title,authors,formats,cover", "--search", query],
        root,
    )
    try:
        entries = json.loads(raw)
        if not isinstance(entries, list):
            raise ValueError("expected a list of books")
        for row in entries:
            int(row["id"])
            row["title"]
    except (ValueError, KeyError, TypeError) as exc:
        raise RuntimeError("calibredb returned an invalid book list.") from exc
    return entries, root


def _entry_book(entry: dict, root: Path | None) -> tuple[Book, Path]:
    formats = {Path(path).suffix[1:].upper(): Path(path) for path in entry.get("formats", [])}
    paths = list(formats.values())
    if entry.get("cover"):
        paths.append(Path(entry["cover"]))
    if not paths:
        raise ValueError(f"No local files found for book {entry['id']}: {entry['title']}")
    directory = paths[0].parent
    # Pin the default library for subsequent calls, even if the GUI preference changes.
    if root is None:
        root = next((p for p in directory.parents if (p / "metadata.db").is_file()), None)
        if root is None:
            raise ValueError(f"Could not locate the Calibre library containing {directory}")
    book = Book(int(entry["id"]), entry["title"], entry.get("authors", ""), formats, directory)
    return book, root


def select_epub(query: str, library: str | None) -> tuple[Book, Path, Path]:
    entries, root = _list_entries(query, library)
    if not entries:
        raise ValueError(f"No Calibre books match: {query}")
    labels = [f"{row['id']} | {row['title']} | {row.get('authors', '')}" for row in entries]
    book, root = _entry_book(entries[_pick(labels, "Book> ")], root)
    candidates = sorted(
        (
            path for path in book.directory.iterdir()
            if path.is_file() and path.suffix.lower() == ".epub"
            and not path.name.lower().endswith(".original.epub")
        ),
        key=lambda path: path.name.casefold(),
    )
    if not candidates:
        raise ValueError(f"No EPUB files found for book {book.id}: {book.title}")
    source = candidates[_pick([path.name for path in candidates], "EPUB> ")]
    return book, source, root


def normalize_calibre(
    query: str,
    library: str | None = None,
    *,
    all_books: bool = False,
    dry_run: bool = False,
    preserve_publisher_css: bool = False,
    verbose: bool = False,
) -> int:
    if all_books:
        return _normalize_all(
            library, dry_run=dry_run,
            preserve_publisher_css=preserve_publisher_css, verbose=verbose,
        )
    book, source, root = select_epub(query, library)
    print(f"Selected {book.id}: {book.title} | {book.authors}")
    _normalize_book(
        book, source, root, dry_run=dry_run,
        preserve_publisher_css=preserve_publisher_css, verbose=verbose,
    )
    return 0


def _normalize_all(
    library: str | None,
    *,
    dry_run: bool,
    preserve_publisher_css: bool,
    verbose: bool,
) -> int:
    # Query the database, not filenames: ORIGINAL_EPUB is a distinct registered format.
    entries, root = _list_entries('formats:"=EPUB" and not formats:"=ORIGINAL_EPUB"', library)
    if not entries:
        print("No books with EPUB and without ORIGINAL_EPUB need processing.")
        return 0
    failed: list[int] = []
    for index, entry in enumerate(entries, 1):
        print(f"[{index}/{len(entries)}] {entry['title']} | {entry.get('authors', '')}", flush=True)
        try:
            book, root = _entry_book(entry, root)
            source = book.formats.get("EPUB")
            if source is None:
                raise ValueError("Registered EPUB file is missing.")
            _normalize_book(
                book, source, root, dry_run=dry_run,
                preserve_publisher_css=preserve_publisher_css, verbose=verbose,
            )
        except Exception as exc:
            failed.append(int(entry["id"]))
            print(f"Book {entry['id']} ({entry['title']}): error: {exc}", file=sys.stderr)
    action = "previewed" if dry_run else "processed"
    print(f"Batch complete: {len(entries) - len(failed)} {action}, {len(failed)} failed.")
    if failed:
        print(f"Failed book IDs: {', '.join(map(str, failed))}", file=sys.stderr)
    return int(bool(failed))


def _normalize_book(
    book: Book,
    source: Path,
    root: Path,
    *,
    dry_run: bool,
    preserve_publisher_css: bool,
    verbose: bool,
) -> None:
    print(f"EPUB: {source}")
    current = book.formats.get("EPUB", source)
    original = book.formats.get("ORIGINAL_EPUB")
    if dry_run:
        preview = preview_epub_changes(source, preserve_publisher_css=preserve_publisher_css)
        print(
            f"[dry run] {preview.content_documents} document(s), "
            f"{preview.stylesheets_and_fonts} stylesheet/font entry(s) to remove, "
            f"{preview.images_preserved} image(s) preserved"
        )
        if original:
            print(f"Would keep ORIGINAL_EPUB: {original}")
        else:
            print(f"Would back up as ORIGINAL_EPUB: {current}")
        print(f"Would replace EPUB on Calibre book {book.id} using add_format.")
        for warning in preview.warnings + preview.image_diagnostics:
            print(f"warning: {warning}", file=sys.stderr)
        return

    def progress(message: str) -> None:
        print(message, file=sys.stderr)

    with tempfile.TemporaryDirectory(prefix="epub-normalize-calibre-") as tmp:
        work = Path(tmp)
        backup = work / current.with_suffix(".original_epub").name
        if original is None:
            shutil.copyfile(current, backup)
        result = optimize_epub(
            source,
            work,
            output_filename=current.name,
            preserve_publisher_css=preserve_publisher_css,
            progress=progress if verbose else None,
        )
        # Only change the library after normalization succeeds. Register the backup first.
        if original is None:
            run_calibredb(["add_format", str(book.id), str(backup), "--dont-replace"], root)
            print("Saved ORIGINAL_EPUB backup in Calibre.")
        else:
            print(f"Kept existing ORIGINAL_EPUB: {original}")
        try:
            run_calibredb(["add_format", str(book.id), str(result.output_path)], root)
        except RuntimeError as exc:
            raise RuntimeError(
                f"{exc}\nThe ORIGINAL_EPUB backup is retained in Calibre."
            ) from exc
    print(f"Updated Calibre book {book.id}. Formats: EPUB, ORIGINAL_EPUB.")
    print(f"  EPUBCheck: {result.validation_outcome}")
    for warning in result.warnings + result.image_diagnostics:
        print(f"warning: {warning}", file=sys.stderr)
