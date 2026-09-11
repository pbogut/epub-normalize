"""Command-line interface for the EPUB normalization engine."""

import argparse
import os
import sys
import tempfile
from pathlib import Path

from epub_optimizer import __version__
from epub_optimizer.calibre import normalize_calibre
from epub_optimizer.core import optimize_epub, preview_epub_changes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Normalize EPUB typography locally.")
    parser.add_argument("inputs", nargs="*", type=Path, help="EPUB files to normalize")
    parser.add_argument(
        "--calibre", metavar="QUERY", nargs="?", const="",
        help="search a Calibre library and pick a book, or use with --all",
    )
    parser.add_argument(
        "--all", action="store_true", help="with --calibre, process EPUBs without ORIGINAL_EPUB"
    )
    parser.add_argument(
        "--with-library", metavar="PATH", help="local Calibre library, with --calibre"
    )
    outputs = parser.add_mutually_exclusive_group()
    outputs.add_argument("-o", "--output", type=Path, help="output filename, for a single input")
    outputs.add_argument("-d", "--output-dir", type=Path, help="directory for normalized books")
    parser.add_argument(
        "-n", "--dry-run", action="store_true", help="preview changes without writing"
    )
    parser.add_argument("-f", "--force", action="store_true", help="replace existing output files")
    parser.add_argument("-v", "--verbose", action="store_true", help="show processing steps")
    parser.add_argument(
        "--preserve-publisher-css",
        action="store_true",
        help="keep publisher styles and fonts alongside the normalization stylesheet",
    )
    parser.add_argument("--version", action="version", version=f"epub-normalize {__version__}")
    args = parser.parse_args(argv)
    if args.all and args.calibre is None:
        parser.error("--all requires --calibre")
    if args.calibre is not None:
        if args.all and args.calibre:
            parser.error("use --calibre --all without a search query")
        if not args.all and not args.calibre.strip():
            parser.error("--calibre requires a non-empty search query or --all")
        if args.inputs or args.output or args.output_dir or args.force:
            parser.error(
                "--calibre cannot be combined with input files, --output, --output-dir or --force"
            )
    else:
        if not args.inputs:
            parser.error("provide EPUB files or --calibre QUERY")
        if args.with_library is not None:
            parser.error("--with-library requires --calibre")
    if args.output and len(args.inputs) != 1:
        parser.error("--output requires exactly one input; use --output-dir for multiple books")
    if args.output and args.output.suffix.lower() != ".epub":
        parser.error("--output must have an .epub extension")
    try:
        if args.calibre is not None:
            try:
                return normalize_calibre(
                    args.calibre,
                    args.with_library,
                    all_books=args.all,
                    dry_run=args.dry_run,
                    preserve_publisher_css=args.preserve_publisher_css,
                    verbose=args.verbose,
                )
            except Exception as exc:
                print(f"error: {exc}", file=sys.stderr)
                return 1
        jobs = _plan_outputs(args, parser)
        failed = False
        for source, destination in jobs:
            try:
                _process(source, destination, args)
            except Exception as exc:
                # A malformed book must not prevent the rest of a batch from running.
                print(f"{source}: error: {exc}", file=sys.stderr)
                failed = True
        return int(failed)
    except KeyboardInterrupt:
        print("Interrupted.", file=sys.stderr)
        return 130


def _plan_outputs(
    args: argparse.Namespace, parser: argparse.ArgumentParser
) -> list[tuple[Path, Path]]:
    jobs = []
    destinations: set[Path] = set()
    try:
        inputs = {source.resolve() for source in args.inputs}
        for source in args.inputs:
            destination = args.output or (args.output_dir or source.parent) / (
                f"{source.stem}-normalized.epub"
            )
            resolved = destination.resolve()
            if resolved in inputs or (
                destination.exists()
                and any(item.exists() and destination.samefile(item) for item in inputs)
            ):
                parser.error(f"output would overwrite an input file: {destination}")
            if resolved in destinations:
                parser.error(f"multiple inputs would write to the same output: {destination}")
            destinations.add(resolved)
            jobs.append((source, destination))
    except (OSError, RuntimeError) as exc:
        parser.error(str(exc))
    return jobs


def _process(source: Path, destination: Path, args: argparse.Namespace) -> None:
    if args.dry_run:
        preview = preview_epub_changes(source, preserve_publisher_css=args.preserve_publisher_css)
        print(f"{source} -> {destination} [dry run]")
        print(
            f"  {preview.content_documents} document(s), "
            f"{preview.stylesheets_and_fonts} stylesheet/font entry(s) to remove, "
            f"{preview.images_preserved} image(s) preserved"
        )
        for warning in preview.warnings + preview.image_diagnostics:
            print(f"{source}: warning: {warning}", file=sys.stderr)
        return

    if not args.force and (destination.exists() or destination.is_symlink()):
        raise FileExistsError(f"Output already exists: {destination}. Use --force to replace it.")
    if not source.is_file():
        raise FileNotFoundError("Input file does not exist or is not a regular file.")
    destination.parent.mkdir(parents=True, exist_ok=True)

    def progress(message: str) -> None:
        print(f"{source}: {message}", file=sys.stderr)

    with tempfile.TemporaryDirectory(prefix=".epub-normalize-", dir=destination.parent) as tmp:
        result = optimize_epub(
            source,
            Path(tmp),
            output_filename=destination.name,
            preserve_publisher_css=args.preserve_publisher_css,
            progress=progress if args.verbose else None,
        )
        if args.force:
            os.replace(result.output_path, destination)
        else:
            # Atomic publication without overwriting, even if another process creates the output.
            os.link(result.output_path, destination)
    print(f"{source} -> {destination}")
    print(
        f"  {result.content_documents_processed} document(s), "
        f"{result.stylesheets_replaced} stylesheet/font entry(s) removed, "
        f"{result.images_preserved} image(s) preserved"
    )
    print(f"  EPUBCheck: {result.validation_outcome}")
    for warning in result.warnings + result.image_diagnostics:
        print(f"{source}: warning: {warning}", file=sys.stderr)
