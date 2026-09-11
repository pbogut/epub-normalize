# epub-normalize

Normalize EPUB typography from the command line, using the processing engine
from [HenryBaby/epub-optimizer](https://github.com/HenryBaby/epub-optimizer).
Runs locally with Python 3.12 or newer. The only runtime Python dependencies
are `lxml` and `defusedxml`.

## Install

From this checkout, install with [pipx](https://pipx.pypa.io/):

```sh
pipx install .
```

To update an existing pipx installation after changing this checkout:

```sh
pipx install --force .
```

Or use a virtual environment:

```sh
python3 -m venv .venv
.venv/bin/python -m pip install .
.venv/bin/epub-normalize book.epub
```

## Usage

```sh
# Write book-normalized.epub alongside the original
epub-normalize book.epub

# Normalize several books into a separate directory
epub-normalize *.epub --output-dir normalized/

# Choose an output filename
epub-normalize book.epub -o cleaned.epub

# Inspect what would change, without writing an output book
epub-normalize book.epub --dry-run

# Replace an existing output and show processing steps
epub-normalize book.epub --force --verbose

# Keep publisher styles and fonts alongside the normalization stylesheet
epub-normalize book.epub --preserve-publisher-css
```

The shell expands `*.epub`. Quote filenames containing spaces. Input arguments
must be files; directories are not scanned recursively. Use a separate output
directory when repeatedly processing a batch so its outputs aren't included in
the next glob.

The command preserves source files and refuses to replace an existing output
unless you pass `--force`. Even with `--force`, an output cannot be any of the
input files. Conflicting output names within a batch are rejected before
processing starts. Output archives are published atomically once processing
succeeds, so a failed run leaves an existing output intact.

If a book fails, the command reports the error and continues with the rest.
Exit codes are `0` for success, `1` if any book failed, `2` for invalid command
arguments or conflicting output paths, and `130` for an interrupted run.

## Calibre library

Search your default Calibre library and normalize a book in place:

```sh
epub-normalize --calibre 'Imperium Czerni'
```

This needs `calibredb` from Calibre on your PATH. When several books match,
`fzf` lets you pick one by title, author, and ID. If the book's directory has
several EPUB files, a second picker selects the input. Single matches are
selected automatically, so `fzf` is only needed for multiple matches.
Press Esc or Ctrl-C to cancel without importing anything.

The command normalizes the selected EPUB in a temporary directory, then calls
`calibredb add_format` to register two formats on the same book:

| Format | Contents |
| --- | --- |
| `EPUB` | Normalized book |
| `ORIGINAL_EPUB` | Original EPUB before replacement |

An existing `ORIGINAL_EPUB` stays untouched on later runs. If you select an
alternate EPUB from the directory, the backup preserves the book's currently
registered EPUB. When the book has no registered EPUB, it backs up the selected
file instead. Backup files are excluded from the EPUB picker.

Calibre manages the filenames and library records. The backup uses the extension
`.original_epub`, so both formats appear in Calibre. The command preserves the
book's ID, tags, and library metadata. Other files already in the directory are
left in place.

Close the Calibre GUI before running this command against a local library.
If Calibre reports that the library is busy, the command displays that error.
Only local libraries are supported.

```sh
# Choose a book and preview changes without importing any formats
epub-normalize --calibre 'Imperium Czerni' --dry-run

# Use Calibre's search syntax
epub-normalize --calibre 'title:"Imperium Czerni"' --verbose

# Use another library, supplying its actual directory
epub-normalize --calibre 'Imperium Czerni' --with-library "$HOME/Calibre Library"
```

`--preserve-publisher-css` also works in Calibre mode. File input arguments,
`--output`, `--output-dir`, and `--force` are not used with `--calibre`; this mode
always replaces the selected book's EPUB after saving or retaining its backup.
No import happens if normalization fails. If saving the backup fails, the EPUB
replacement is not attempted. If replacement fails, the registered
`ORIGINAL_EPUB` remains available in Calibre.

### Process all books without backups

```sh
epub-normalize --calibre --all

# Preview the same batch without importing anything
epub-normalize --calibre --all --dry-run
```

Batch mode selects books with a registered `EPUB` and no registered
`ORIGINAL_EPUB`. It uses each book's registered EPUB directly, without opening
`fzf` or selecting other EPUB files from its directory. Format matching is exact;
an `ORIGINAL_EPUB` by itself does not count as an EPUB.

Books run sequentially, with progress and a final processed/failed count. A failed
book does not stop the remaining books. Failed IDs are printed at the end, and
the command exits with `1` if any book failed. An empty batch exits with `0`.
Ctrl-C stops processing and exits with `130`.

`--with-library`, `--verbose`, and `--preserve-publisher-css` also work with
`--all`. Use `--calibre --all` without a search query.

Successful books are skipped on subsequent batch runs because they now have
`ORIGINAL_EPUB`. If backup registration succeeds but EPUB replacement fails,
that book also has `ORIGINAL_EPUB` and will be skipped by the next batch. Retry
it in single-book mode using its reported ID, for example
`epub-normalize --calibre 'id:42'` for book 42. Its existing backup stays intact.

## What changes

The upstream engine supports EPUB 2 and EPUB 3. By default it:

- Replaces publisher CSS with a shared typography stylesheet.
- Removes embedded fonts and obsolete stylesheet entries.
- Normalizes headings, paragraphs, front matter, quotes, and navigation.
- Preserves readable text, metadata, spine order, links, emphasis, and images.
- Repackages the EPUB with its uncompressed `mimetype` entry first.

Image bytes are unchanged. File size may decrease after removing fonts and
styles, but a smaller file is not guaranteed. This tool does not remove DRM.

Each output contains `META-INF/epub-optimizer-report.json` with processing
details and validation results. The internal stylesheet and report names retain
the upstream names.

`--dry-run` inspects the archive and reports document, removable stylesheet/font,
and image counts. It does not run the full rewrite or EPUBCheck, so a successful
preview does not guarantee that normalization will succeed.

## Optional EPUBCheck

Built-in archive and structural checks always run. For additional validation,
install [EPUBCheck](https://www.w3.org/publishing/epubcheck/) and configure either
an executable or the official JAR:

```sh
EPUBCHECK_EXECUTABLE=epubcheck epub-normalize book.epub
EPUBCHECK_JAR=/path/to/epubcheck.jar epub-normalize book.epub
```

The JAR needs Java. `JAVA` selects its executable and `EPUBCHECK_TIMEOUT` sets
the timeout in seconds for each invocation, defaulting to 120.

When configured, EPUBCheck checks both the input and output. The engine attempts
supported repairs and rejects outputs that introduce new EPUBCheck errors.
The command prints one of these outcomes:

- `clean`: no EPUBCheck errors remain.
- `legacy_issues`: pre-existing errors remain, but no new errors were detected.
- `unavailable`: EPUBCheck could not run; only the built-in checks ran.

## Development

```sh
python3 -m venv .venv
.venv/bin/python -m pip install -e '.[dev]'
.venv/bin/pytest
.venv/bin/ruff check .
```

When `calibredb` is installed, the tests also exercise normalization and repeated
imports in a disposable library with isolated Calibre settings. That integration
test is skipped when Calibre is unavailable.

## License

Licensed under AGPL-3.0-only. See [NOTICE.md](NOTICE.md) for upstream attribution.
