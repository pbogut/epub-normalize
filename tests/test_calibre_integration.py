"""Exercise real calibredb calls against an isolated, disposable library."""

import json
import shutil
import subprocess
import zipfile
from pathlib import Path

import pytest
from test_integration_optimize import _write_minimal_epub

from epub_optimizer.cli import main

pytestmark = pytest.mark.skipif(
    shutil.which("calibredb") is None, reason="Calibre is not installed"
)


@pytest.fixture
def calibre_library(tmp_path, monkeypatch):
    config = tmp_path / "config"
    config.mkdir()
    library = tmp_path / "library"
    library.mkdir()
    monkeypatch.setenv("CALIBRE_CONFIG_DIRECTORY", str(config))
    monkeypatch.setenv("QT_QPA_PLATFORM", "offscreen")
    monkeypatch.setenv("EPUBCHECK_EXECUTABLE", str(tmp_path / "no-epubcheck"))
    def db(*args):
        result = subprocess.run(
            ["calibredb", *args, "--with-library", str(library)],
            capture_output=True, text=True, check=True,
        )
        return result.stdout

    return library, db


def test_real_calibre_retains_original_and_book_record_on_rerun(tmp_path, calibre_library):
    library, db = calibre_library
    source = tmp_path / "input.epub"
    _write_minimal_epub(source)
    db("add", str(source), "--title", "Imperium Czerni", "--tags", "keep-this-tag")
    before = json.loads(db("list", "--for-machine", "--fields", "title,tags,formats"))[0]
    registered_source = Path(before["formats"][0])
    original_bytes = registered_source.read_bytes()

    arguments = ["--calibre", "Imperium Czerni", "--with-library", str(library)]
    assert main([*arguments, "--dry-run"]) == 0
    assert registered_source.read_bytes() == original_bytes
    assert len(json.loads(db("list", "--for-machine", "--fields", "formats"))[0]["formats"]) == 1

    for _ in range(2):
        assert main(arguments) == 0
        records = json.loads(db("list", "--for-machine", "--fields", "title,tags,formats"))
        assert len(records) == 1
        record = records[0]
        assert record["id"] == before["id"]
        assert record["title"] == "Imperium Czerni"
        assert record["tags"] == ["keep-this-tag"]
        formats = {Path(path).suffix: Path(path) for path in record["formats"]}
        assert set(formats) == {".epub", ".original_epub"}
        assert formats[".original_epub"].read_bytes() == original_bytes
        assert formats[".epub"].read_bytes() != original_bytes
        with zipfile.ZipFile(formats[".epub"]) as archive:
            assert "OEBPS/Styles/epub-optimizer.css" in archive.namelist()
            assert "OEBPS/Styles/old.css" not in archive.namelist()
        ebook_files = [
            path for path in registered_source.parent.iterdir()
            if path.suffix in {".epub", ".original_epub"}
        ]
        assert len(ebook_files) == 2


def test_real_batch_filters_formats_and_skips_finished_books(
    tmp_path, calibre_library, monkeypatch, capsys,
):
    from epub_optimizer import calibre

    library, db = calibre_library
    source = tmp_path / "input.epub"
    _write_minimal_epub(source)
    for title in ["Eligible one", "Eligible two", "Already backed up", "Original only"]:
        db("add", str(source), "--title", title)
    text = tmp_path / "text.txt"
    text.write_text("No EPUB here.")
    db("add", str(text), "--title", "Text only")
    records = json.loads(db("list", "--for-machine", "--fields", "title,formats"))
    backup = tmp_path / "backup.original_epub"
    shutil.copyfile(source, backup)
    for row in records:
        if row["title"] in {"Already backed up", "Original only"}:
            db("add_format", str(row["id"]), str(backup))
        if row["title"] == "Original only":
            db("remove_format", str(row["id"]), "EPUB")

    def snapshot():
        rows = json.loads(db("list", "--for-machine", "--fields", "title,formats"))
        return {
            row["title"]: {Path(path).suffix: Path(path).read_bytes() for path in row["formats"]}
            for row in rows
        }

    def unexpected_picker(*args):
        pytest.fail("Batch mode must not open a picker")

    monkeypatch.setattr(calibre, "_pick", unexpected_picker)
    # An extra, unregistered EPUB must not be selected or affect batch eligibility.
    registered = Path(records[0]["formats"][0])
    extra = registered.with_stem("Unregistered alternate")
    extra.write_bytes(b"not an epub")
    before = snapshot()
    arguments = ["--calibre", "--all", "--with-library", str(library)]
    assert main([*arguments, "--dry-run"]) == 0
    assert "2 previewed, 0 failed" in capsys.readouterr().out
    assert snapshot() == before
    assert main(arguments) == 0
    assert "2 processed, 0 failed" in capsys.readouterr().out
    after = snapshot()
    for title in ["Eligible one", "Eligible two"]:
        assert set(after[title]) == {".epub", ".original_epub"}
        assert after[title][".original_epub"] == before[title][".epub"]
        assert after[title][".epub"] != before[title][".epub"]
    for title in ["Already backed up", "Original only", "Text only"]:
        assert after[title] == before[title]
    assert extra.read_bytes() == b"not an epub"
    assert main(arguments) == 0
    assert "No books" in capsys.readouterr().out
    assert snapshot() == after
