import json
import subprocess
from pathlib import Path

import pytest
from test_integration_optimize import _write_minimal_epub

from epub_optimizer import calibre
from epub_optimizer.cli import main
from epub_optimizer.epubcheck import EpubCheckResult, EpubCheckRunner


@pytest.fixture(autouse=True)
def no_external_epubcheck(monkeypatch):
    monkeypatch.setattr(
        EpubCheckRunner, "check", lambda self, path: EpubCheckResult(False, "unavailable")
    )


@pytest.fixture
def library(tmp_path):
    root = tmp_path / "Calibre Library"
    directory = root / "Author" / "Imperium Czerni (42)"
    directory.mkdir(parents=True)
    (root / "metadata.db").touch()
    source = directory / "Imperium Czerni - Author.epub"
    source.touch()
    entry = {"id": 42, "title": "Imperium Czerni", "authors": "Author", "formats": [str(source)]}
    return root, source, entry


def test_single_match_selects_automatically_and_pins_library(library, monkeypatch):
    root, source, entry = library
    calls = []

    def run(args, selected_library):
        calls.append((args, selected_library))
        return json.dumps([entry])

    monkeypatch.setattr(calibre, "run_calibredb", run)
    book, selected, selected_library = calibre.select_epub("Imperium Czerni", None)
    assert book.id == 42
    assert selected == source
    assert selected_library == root
    assert calls[0][0][-2:] == ["--search", "Imperium Czerni"]
    assert calls[0][1] is None


def test_multiple_books_and_epubs_use_two_pickers(library, monkeypatch):
    root, source, entry = library
    second = source.with_stem("Other edition")
    second.touch()
    source.with_suffix(".original_epub").touch()
    source.with_suffix(".original.epub").touch()
    entries = [{**entry, "id": 1, "title": "Different book"}, entry]
    monkeypatch.setattr(calibre, "run_calibredb", lambda *args: json.dumps(entries))
    prompts = []

    def pick(labels, prompt):
        prompts.append((labels, prompt))
        return 1

    monkeypatch.setattr(calibre, "_pick", pick)
    book, selected, _ = calibre.select_epub("Imperium", str(root))
    assert book.id == 42
    assert selected == second
    assert prompts[0][1] == "Book> "
    assert prompts[1] == ([source.name, second.name], "EPUB> ")


def test_no_matches(library, monkeypatch):
    monkeypatch.setattr(calibre, "run_calibredb", lambda *args: "[]")
    with pytest.raises(ValueError, match="No Calibre books match"):
        calibre.select_epub("unknown", None)


def test_invalid_library_is_not_created(tmp_path):
    missing = tmp_path / "not-a-library"
    with pytest.raises(ValueError, match="metadata.db is missing"):
        calibre.select_epub("book", str(missing))
    assert not missing.exists()


def test_fzf_protocol_ignores_user_defaults(monkeypatch):
    monkeypatch.setenv("FZF_DEFAULT_OPTS", "--multi --filter=wrong")
    monkeypatch.setenv("FZF_DEFAULT_OPTS_FILE", "/wrong")

    def run(command, **kwargs):
        assert "FZF_DEFAULT_OPTS" not in kwargs["env"]
        assert "FZF_DEFAULT_OPTS_FILE" not in kwargs["env"]
        assert kwargs["input"] == "0\tFirst title\n1\tSecond\n"
        return subprocess.CompletedProcess(command, 0, "1\tSecond\n")

    monkeypatch.setattr(subprocess, "run", run)
    assert calibre._pick(["First\ntitle", "Second"], "Book> ") == 1


@pytest.mark.parametrize("status", [1, 130])
def test_picker_cancel(monkeypatch, status):
    monkeypatch.setattr(
        subprocess, "run", lambda *args, **kwargs: subprocess.CompletedProcess(args, status, "")
    )
    with pytest.raises(KeyboardInterrupt):
        calibre._pick(["one", "two"], "Book> ")


def test_normalize_registers_backup_before_replacement(library, monkeypatch, capsys):
    root, source, entry = library
    _write_minimal_epub(source)
    original = source.read_bytes()
    imports = []

    def run(args, selected_library):
        if args[0] == "list":
            return json.dumps([entry])
        assert selected_library == root
        assert args[0:2] == ["add_format", "42"]
        path = Path(args[2])
        imports.append((path.suffix, path.read_bytes(), args[3:]))
        return ""

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "Imperium Czerni"]) == 0
    assert imports[0] == (".original_epub", original, ["--dont-replace"])
    assert imports[1][0] == ".epub"
    assert imports[1][1] != original
    assert imports[1][2] == []
    assert source.read_bytes() == original  # All library writes go through calibredb.
    assert "Updated Calibre book 42" in capsys.readouterr().out


def test_rerun_preserves_existing_original(library, monkeypatch):
    _, source, entry = library
    _write_minimal_epub(source)
    backup = source.with_suffix(".original_epub")
    backup.write_bytes(b"first original")
    entry["formats"].append(str(backup))
    imports = []

    def run(args, root):
        if args[0] == "list":
            return json.dumps([entry])
        imports.append(args)
        return ""

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "Imperium"]) == 0
    assert len(imports) == 1
    assert Path(imports[0][2]).suffix == ".epub"
    assert backup.read_bytes() == b"first original"


def test_selected_alternate_epub_backs_up_registered_epub(library, monkeypatch):
    _, source, entry = library
    source.write_bytes(b"registered original")
    alternate = source.with_stem("Other edition")
    _write_minimal_epub(alternate)
    monkeypatch.setattr(calibre, "_pick", lambda labels, prompt: len(labels) - 1)
    backups = []

    def run(args, root):
        if args[0] == "list":
            return json.dumps([entry])
        if Path(args[2]).suffix == ".original_epub":
            backups.append(Path(args[2]).read_bytes())
        return ""

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "Imperium"]) == 0
    assert backups == [b"registered original"]


@pytest.mark.parametrize("dry_run", [False, True])
def test_invalid_epub_never_imports(library, monkeypatch, dry_run):
    _, source, entry = library
    source.write_bytes(b"not an epub")

    def run(args, root):
        assert args[0] == "list"
        return json.dumps([entry])

    monkeypatch.setattr(calibre, "run_calibredb", run)
    args = ["--calibre", "Imperium"] + (["--dry-run"] if dry_run else [])
    assert main(args) == 1
    assert source.read_bytes() == b"not an epub"


def test_dry_run_never_imports_or_creates_backup(library, monkeypatch, capsys):
    root, source, entry = library
    _write_minimal_epub(source)
    before = source.read_bytes()

    def run(args, selected_library):
        assert args[0] == "list"
        return json.dumps([entry])

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "Imperium", "--with-library", str(root), "--dry-run"]) == 0
    assert list(source.parent.iterdir()) == [source]
    assert source.read_bytes() == before
    assert "Would back up as ORIGINAL_EPUB" in capsys.readouterr().out


@pytest.mark.parametrize("failure", ["backup", "replacement"])
def test_import_failure_stops_and_reports_error(library, monkeypatch, capsys, failure):
    _, source, entry = library
    _write_minimal_epub(source)
    imports = []

    def run(args, root):
        if args[0] == "list":
            return json.dumps([entry])
        imports.append(args)
        if failure == "backup" or len(imports) == 2:
            raise RuntimeError("calibredb add_format failed: library locked")
        return ""

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "Imperium"]) == 1
    assert len(imports) == (1 if failure == "backup" else 2)
    captured = capsys.readouterr()
    assert "library locked" in captured.err
    assert "Updated Calibre" not in captured.out
    if failure == "replacement":
        assert "backup is retained" in captured.err


def test_cancel_from_cli_never_imports(library, monkeypatch):
    _, _, entry = library
    monkeypatch.setattr(calibre, "run_calibredb", lambda *args: json.dumps([entry, entry]))

    def cancel(*args):
        raise KeyboardInterrupt

    monkeypatch.setattr(calibre, "_pick", cancel)
    assert main(["--calibre", "Imperium"]) == 130


@pytest.mark.parametrize("args", [
    ["--calibre", ""],
    ["--calibre", "book", "input.epub"],
    ["--calibre", "book", "--output", "output.epub"],
    ["--calibre", "book", "--output-dir", "output"],
    ["--calibre", "book", "--force"],
    ["input.epub", "--with-library", "library"],
])
def test_invalid_cli_combinations(args):
    with pytest.raises(SystemExit) as exc:
        main(args)
    assert exc.value.code == 2


def test_calibredb_arguments_are_not_shell_interpreted(monkeypatch, tmp_path):
    query = 'title:"Imperium Czerni"; $(anything)'

    def run(command, **kwargs):
        assert command == ["calibredb", "list", "--search", query, "--with-library", str(tmp_path)]
        assert kwargs.get("shell", False) is False
        return subprocess.CompletedProcess(command, 0, "[]", "")

    monkeypatch.setattr(subprocess, "run", run)
    assert calibre.run_calibredb(["list", "--search", query], tmp_path) == "[]"


@pytest.mark.parametrize("dry_run", [False, True])
def test_batch_uses_registered_epubs_without_pickers(library, monkeypatch, capsys, dry_run):
    root, source, entry = library
    _write_minimal_epub(source)
    original = source.read_bytes()
    source.with_stem("An unrelated alternative").write_bytes(b"not the registered EPUB")
    imports = []

    def run(args, selected_library):
        if args[0] == "list":
            assert args[-2:] == ["--search", 'formats:"=EPUB" and not formats:"=ORIGINAL_EPUB"']
            return json.dumps([entry])
        assert selected_library == root
        imports.append((Path(args[2]).suffix, Path(args[2]).read_bytes()))
        return ""

    def unexpected_picker(*args):
        pytest.fail("Batch mode must not open a picker")

    monkeypatch.setattr(calibre, "run_calibredb", run)
    monkeypatch.setattr(calibre, "_pick", unexpected_picker)
    args = ["--calibre", "--all"] + (["--dry-run"] if dry_run else [])
    assert main(args) == 0
    if dry_run:
        assert imports == []
    else:
        assert imports[0] == (".original_epub", original)
        assert imports[1][0] == ".epub"
        assert imports[1][1] != original
    captured = capsys.readouterr()
    assert "[1/1] Imperium Czerni" in captured.out
    assert f"1 {'previewed' if dry_run else 'processed'}, 0 failed" in captured.out


@pytest.mark.parametrize("failure", ["invalid_epub", "backup", "replacement"])
def test_batch_continues_after_failure(library, monkeypatch, capsys, failure):
    _, source, entry = library
    _write_minimal_epub(source)
    other = source.with_stem("Second book")
    _write_minimal_epub(other)
    second = {**entry, "id": 43, "title": "Second book", "formats": [str(other)]}
    if failure == "invalid_epub":
        source.write_bytes(b"invalid")
    imports = []

    def run(args, root):
        if args[0] == "list":
            return json.dumps([entry, second])
        imports.append((args[1], Path(args[2]).suffix))
        if args[1] == "42" and (
            failure == "backup" or failure == "replacement" and Path(args[2]).suffix == ".epub"
        ):
            raise RuntimeError("import failed")
        return ""

    monkeypatch.setattr(calibre, "run_calibredb", run)
    assert main(["--calibre", "--all"]) == 1
    assert imports[-2:] == [("43", ".original_epub"), ("43", ".epub")]
    captured = capsys.readouterr()
    assert "[2/2] Second book" in captured.out
    assert "1 processed, 1 failed" in captured.out
    assert "Failed book IDs: 42" in captured.err
    if failure == "replacement":
        assert "backup is retained" in captured.err


def test_batch_no_eligible_books_is_success(monkeypatch, capsys):
    monkeypatch.setattr(calibre, "run_calibredb", lambda *args: "[]")
    assert main(["--calibre", "--all"]) == 0
    assert "No books" in capsys.readouterr().out


def test_batch_interrupt_stops_processing(library, monkeypatch):
    _, _, entry = library
    monkeypatch.setattr(calibre, "run_calibredb", lambda *args: json.dumps([entry, entry]))
    calls = []

    def interrupt(*args, **kwargs):
        calls.append(args)
        raise KeyboardInterrupt

    monkeypatch.setattr(calibre, "_normalize_book", interrupt)
    assert main(["--calibre", "--all"]) == 130
    assert len(calls) == 1


@pytest.mark.parametrize("args", [
    ["--all"],
    ["input.epub", "--all"],
    ["--calibre"],
    ["--calibre", "book", "--all"],
    ["--calibre", "--all", "input.epub"],
    ["--calibre", "--all", "--force"],
])
def test_invalid_batch_arguments(args):
    with pytest.raises(SystemExit) as exc:
        main(args)
    assert exc.value.code == 2
