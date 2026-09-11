import json
import os
import subprocess
import sys
import zipfile
from pathlib import Path

import pytest
from test_integration_optimize import _write_minimal_epub

from epub_optimizer.cli import main
from epub_optimizer.epubcheck import EpubCheckResult, EpubCheckRunner


@pytest.fixture(autouse=True)
def no_external_epubcheck(monkeypatch):
    monkeypatch.setattr(
        EpubCheckRunner, "check", lambda self, path: EpubCheckResult(False, "unavailable")
    )


@pytest.fixture
def book(tmp_path):
    source = tmp_path / "A book.epub"
    _write_minimal_epub(source)
    return source


def test_normalize_preserves_original_and_writes_epub(book, capsys):
    original = book.read_bytes()
    assert main([str(book)]) == 0
    assert book.read_bytes() == original
    output = book.with_stem("A book-normalized")
    with zipfile.ZipFile(output) as archive:
        assert archive.infolist()[0].filename == "mimetype"
        assert archive.infolist()[0].compress_type == zipfile.ZIP_STORED
        assert "OEBPS/Styles/old.css" not in archive.namelist()
        assert "OEBPS/Fonts/publisher.woff2" not in archive.namelist()
        assert b"emphasis" in archive.read("OEBPS/Text/chapter.xhtml")
        report = json.loads(archive.read("META-INF/epub-optimizer-report.json"))
        assert report["output_filename"] == output.name
        assert report["validation_outcome"] == "unavailable"
    assert "EPUBCheck: unavailable" in capsys.readouterr().out
    assert not list(book.parent.glob(".epub-normalize-*"))


def test_batch_to_new_output_directory(book, tmp_path):
    second = tmp_path / "other.EPUB"
    _write_minimal_epub(second)
    output = tmp_path / "nested" / "normalized"
    assert main([str(book), str(second), "--output-dir", str(output)]) == 0
    assert sorted(path.name for path in output.iterdir()) == [
        "A book-normalized.epub",
        "other-normalized.epub",
    ]


def test_explicit_output_and_verbose(book, tmp_path, capsys):
    output = tmp_path / "custom.epub"
    assert main([str(book), "-o", str(output), "-v"]) == 0
    assert output.is_file()
    assert "Extracted EPUB" in capsys.readouterr().err


def test_dry_run_does_not_create_output_directory(book, tmp_path, capsys):
    before = book.read_bytes()
    output = tmp_path / "new-directory"
    assert main([str(book), "--dry-run", "--output-dir", str(output)]) == 0
    assert not output.exists()
    assert book.read_bytes() == before
    assert "[dry run]" in capsys.readouterr().out
    assert sorted(path.name for path in tmp_path.iterdir()) == [book.name]


def test_existing_output_requires_force(book):
    output = book.with_stem("A book-normalized")
    output.write_bytes(b"previous output")
    assert main([str(book)]) == 1
    assert output.read_bytes() == b"previous output"
    assert main([str(book), "--force"]) == 0
    assert zipfile.is_zipfile(output)


def test_failed_force_keeps_existing_output_and_cleans_staging(book):
    book.write_bytes(b"not a zip")
    output = book.with_stem("A book-normalized")
    output.write_bytes(b"previous output")
    assert main([str(book), "--force"]) == 1
    assert output.read_bytes() == b"previous output"
    assert not list(book.parent.glob(".epub-normalize-*"))


@pytest.mark.parametrize("alias", ["same", "symlink", "hardlink"])
def test_cannot_overwrite_source_even_with_force(book, tmp_path, alias):
    output = book
    if alias != "same":
        output = tmp_path / "alias.epub"
        if alias == "symlink":
            output.symlink_to(book)
        else:
            output.hardlink_to(book)
    original = book.read_bytes()
    with pytest.raises(SystemExit) as exc:
        main([str(book), "--output", str(output), "--force"])
    assert exc.value.code == 2
    assert book.read_bytes() == original


def test_batch_output_cannot_overwrite_another_input(book):
    other = book.with_stem("A book-normalized")
    _write_minimal_epub(other)
    before = other.read_bytes()
    with pytest.raises(SystemExit) as exc:
        main([str(book), str(other), "--force"])
    assert exc.value.code == 2
    assert other.read_bytes() == before
    assert not other.with_stem("A book-normalized-normalized").exists()


def test_batch_rejects_duplicate_output_names_before_writing(book, tmp_path):
    other_dir = tmp_path / "other"
    other_dir.mkdir()
    other = other_dir / book.name
    _write_minimal_epub(other)
    output = tmp_path / "out"
    with pytest.raises(SystemExit) as exc:
        main([str(book), str(other), "--output-dir", str(output), "--force"])
    assert exc.value.code == 2
    assert not output.exists()


def test_batch_continues_after_invalid_input(book, tmp_path, capsys):
    invalid = tmp_path / "invalid.epub"
    invalid.write_bytes(b"not a zip")
    missing = tmp_path / "missing.epub"
    assert main([str(invalid), str(missing), str(book)]) == 1
    assert book.with_stem("A book-normalized").is_file()
    assert not invalid.with_stem("invalid-normalized").exists()
    errors = capsys.readouterr().err
    assert "invalid.epub: error:" in errors
    assert "missing.epub: error:" in errors
    assert "Traceback" not in errors


def test_preserve_publisher_css(book):
    assert main([str(book), "--preserve-publisher-css"]) == 0
    with zipfile.ZipFile(book.with_stem("A book-normalized")) as archive:
        assert "OEBPS/Styles/old.css" in archive.namelist()
        assert "OEBPS/Fonts/publisher.woff2" in archive.namelist()
        assert "OEBPS/Styles/epub-optimizer.css" in archive.namelist()


def test_concurrent_output_is_not_overwritten(book, monkeypatch):
    link = os.link

    def create_output_first(source, destination):
        Path(destination).write_bytes(b"another process wrote this")
        link(source, destination)

    monkeypatch.setattr(os, "link", create_output_first)
    assert main([str(book)]) == 1
    assert book.with_stem("A book-normalized").read_bytes() == b"another process wrote this"
    assert not list(book.parent.glob(".epub-normalize-*"))


@pytest.mark.parametrize(
    "arguments",
    [[], ["a.epub", "b.epub", "-o", "out.epub"], ["a.epub", "-o", "out.txt"]],
)
def test_usage_errors(arguments):
    with pytest.raises(SystemExit) as exc:
        main(arguments)
    assert exc.value.code == 2


def test_installed_command_runs_outside_repository(book, tmp_path):
    # Use the installed console script, without pytest's source-path injection.
    command = Path(sys.executable).parent / "epub-normalize"
    env = {key: value for key, value in os.environ.items() if key != "PYTHONPATH"}
    env["EPUBCHECK_EXECUTABLE"] = str(tmp_path / "no-epubcheck")
    run = subprocess.run(
        [str(command), str(book)], cwd=tmp_path, env=env, capture_output=True, text=True
    )
    assert run.returncode == 0, run.stderr
    assert "EPUBCheck: unavailable" in run.stdout
    assert zipfile.is_zipfile(book.with_stem("A book-normalized"))
