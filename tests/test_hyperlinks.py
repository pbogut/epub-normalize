import json
import zipfile
from pathlib import Path

import pytest
from lxml import etree

from epub_optimizer.core import optimize_epub, validate_epub_details
from epub_optimizer.epubcheck import EpubCheckResult, EpubCheckRunner
from epub_optimizer.errors import InvalidEpubError
from epub_optimizer.repair import repair_hyperlinks


def write_documents(root: Path, contents: dict[str, str]):
    items = []
    for name, body in contents.items():
        path = root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Test</title></head>'
            f"<body>{body}</body></html>", encoding="utf-8",
        )
        items.append(
            etree.Element("item", href=name, attrib={"media-type": "application/xhtml+xml"})
        )
    return items


def test_repair_root_footnote_return_link(tmp_path):
    items = write_documents(tmp_path, {
        "przypisy.html": '<p><a href="../Text/chapter-05.html#sdfootnote1anc">Return</a></p>',
        "OEBPS/Text/chapter-05.html": '<p id="sdfootnote1anc">Text</p>',
    })
    actions = repair_hyperlinks(tmp_path, "", items)
    root = etree.parse(str(tmp_path / "przypisy.html"))
    assert root.xpath("//@href") == ["OEBPS/Text/chapter-05.html#sdfootnote1anc"]
    assert len(actions) == 1
    assert "przypisy.html" in actions[0]
    assert "Repaired hyperlink" in actions[0]
    assert repair_hyperlinks(tmp_path, "", items) == []


def test_remove_stale_windows_link_preserves_text_and_markup(tmp_path):
    items = write_documents(tmp_path, {
        "index.xhtml": '<p>Before <a id="top-link" href="/C:/Documents%20and%20Settings/heg/'
                       'Pulpit/harry%20potter/#toc"><em>[top]</em></a> after.</p>',
    })
    actions = repair_hyperlinks(tmp_path, "", items)
    root = etree.parse(str(tmp_path / "index.xhtml"))
    assert root.xpath("//@href") == []
    assert root.xpath("string(//*[local-name()='p'])") == "Before [top] after."
    assert root.xpath("//*[@id='top-link']/*[local-name()='em']/text()") == ["[top]"]
    assert "text preserved" in actions[0]


def test_ambiguous_targets_are_not_guessed(tmp_path):
    items = write_documents(tmp_path, {
        "index.xhtml": '<p><a href="../old/chapter.xhtml#note">Note</a></p>',
        "one/chapter.xhtml": '<p id="note">One</p>',
        "two/chapter.xhtml": '<p id="note">Two</p>',
    })
    repair_hyperlinks(tmp_path, "", items)
    assert etree.parse(str(tmp_path / "index.xhtml")).xpath("//@href") == []


def test_valid_local_and_external_links_and_resource_attributes_unchanged(tmp_path):
    items = write_documents(tmp_path, {
        "Text/chapter.xhtml": '<p id="local"><a href="#local">Local</a>'
                              '<a href="../other.xhtml#note">Other</a>'
                              '<a href="https://example.com/missing#note">Web</a>'
                              '<a href="//example.com/">Web</a><a href="mailto:a@b.com">Mail</a>'
                              '<a href="">Top</a><img src="../../outside.png"/></p>',
        "other.xhtml": '<p id="note">Note</p>',
    })
    before = (tmp_path / "Text/chapter.xhtml").read_bytes()
    assert repair_hyperlinks(tmp_path, "", items) == []
    assert (tmp_path / "Text/chapter.xhtml").read_bytes() == before


def test_unique_anchor_recovers_moved_document_and_encodes_filename(tmp_path):
    items = write_documents(tmp_path, {
        "Text/notes.xhtml": '<p><a href="../../old/book.html?q=1#note%20one">Return</a></p>',
        "Nowe części/rozdział 1.xhtml": '<p><a name="note one">Text</a></p>',
    })
    repair_hyperlinks(tmp_path, "", items)
    root = etree.parse(str(tmp_path / "Text/notes.xhtml"))
    assert root.xpath("//@href") == [
        "../Nowe%20cz%C4%99%C5%9Bci/rozdzia%C5%82%201.xhtml?q=1#note%20one"
    ]


def test_unique_anchor_recovers_link_after_document_was_split(tmp_path):
    items = write_documents(tmp_path, {
        "index.xhtml": '<p><a href="chapter.xhtml#note">Note</a></p>',
        "chapter.xhtml": '<p id="other">Chapter</p>',
        "unrelated.xhtml": '<p id="note">Unrelated note</p>',
    })
    repair_hyperlinks(tmp_path, "", items)
    assert etree.parse(str(tmp_path / "index.xhtml")).xpath("//@href") == ["unrelated.xhtml#note"]


def test_filename_disambiguates_repeated_anchor_names(tmp_path):
    items = write_documents(tmp_path, {
        "notes.xhtml": '<p><a href="../old/chapter.html#note">Return</a></p>',
        "Text/chapter.html": '<p id="note">Correct</p>',
        "Text/other.html": '<p id="note">Other</p>',
    })
    repair_hyperlinks(tmp_path, "", items)
    assert etree.parse(str(tmp_path / "notes.xhtml")).xpath("//@href") == ["Text/chapter.html#note"]


def test_unique_filename_repairs_link_without_fragment(tmp_path):
    items = write_documents(tmp_path, {
        "notes.xhtml": '<p><a href="../old/chapter.html">Chapter</a></p>',
        "Text/chapter.html": '<p>Text</p>',
    })
    repair_hyperlinks(tmp_path, "", items)
    assert etree.parse(str(tmp_path / "notes.xhtml")).xpath("//@href") == ["Text/chapter.html"]


def test_absolute_path_never_reads_outside_archive(tmp_path, monkeypatch):
    outside = tmp_path / "outside.xhtml"
    outside.write_text('<html><body id="note">Private content</body></html>')
    work = tmp_path / "work"
    items = write_documents(work, {
        "index.xhtml": f'<p><a href="{outside.as_uri().removeprefix("file://")}#note">Note</a></p>',
    })
    parse = etree.parse

    def guarded_parse(path, *args, **kwargs):
        assert Path(path).resolve().is_relative_to(work)
        return parse(path, *args, **kwargs)

    monkeypatch.setattr(etree, "parse", guarded_parse)
    actions = repair_hyperlinks(work, "", items)
    assert len(actions) == 1
    assert "Removed broken hyperlink" in actions[0]


def _write_book(tmp_path, bodies):
    work = tmp_path / "documents"
    items = write_documents(work, bodies)
    package = etree.Element(
        "package", nsmap={None: "http://www.idpf.org/2007/opf"},
        attrib={"version": "3.0", "unique-identifier": "id"},
    )
    metadata = etree.SubElement(package, "metadata")
    for tag, value in [("identifier", "urn:test-links"), ("title", "Links"), ("language", "en")]:
        child = etree.SubElement(metadata, f"{{http://purl.org/dc/elements/1.1/}}{tag}")
        child.text = value
        if tag == "identifier":
            child.set("id", "id")
    manifest = etree.SubElement(package, "manifest")
    spine = etree.SubElement(package, "spine")
    for index, item in enumerate(items):
        item.set("id", f"doc{index}")
        manifest.append(item)
        etree.SubElement(spine, "itemref", idref=f"doc{index}")
    path = tmp_path / "book.epub"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        archive.writestr(
            "META-INF/container.xml",
            '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">'
            '<rootfiles><rootfile full-path="content.opf" '
            'media-type="application/oebps-package+xml"/>'
            '</rootfiles></container>',
        )
        archive.writestr("content.opf", etree.tostring(package))
        for name in bodies:
            archive.write(work / name, name)
    return path


def test_normalization_repairs_and_records_links_without_epubcheck(tmp_path, monkeypatch):
    monkeypatch.setattr(
        EpubCheckRunner, "check", lambda *args: EpubCheckResult(False, "unavailable")
    )
    source = _write_book(tmp_path, {
        "przypisy.html": '<p><a href="../Text/chapter-05.html#sdfootnote1anc">Return</a></p>',
        "OEBPS/Text/chapter-05.html": '<p id="sdfootnote1anc">Text</p>',
        "index.xhtml": '<p>Before <a href="/C:/Documents%20and%20Settings/heg/#toc">[top]</a>'
                       ' after.</p>',
    })
    original = source.read_bytes()
    assert not validate_epub_details(source).valid
    result = optimize_epub(source, tmp_path / "out")
    assert source.read_bytes() == original
    assert validate_epub_details(result.output_path).valid
    assert len(result.repair_actions) == 2
    assert all(action in result.log for action in result.repair_actions)
    with zipfile.ZipFile(result.output_path) as archive:
        report = json.loads(archive.read("META-INF/epub-optimizer-report.json"))
        assert report["repair_actions"] == result.repair_actions
        assert report["validation_outcome"] == "unavailable"
        notes = etree.fromstring(archive.read("przypisy.html"))
        assert notes.xpath("//*[local-name()='a']/@href") == [
            "OEBPS/Text/chapter-05.html#sdfootnote1anc"
        ]
        top = etree.fromstring(archive.read("index.xhtml"))
        assert top.xpath("//*[local-name()='a']/@href") == []
        assert top.xpath("string(//*[local-name()='body'])").strip() == "Before [top] after."


def test_normalization_still_rejects_unsafe_resource_targets(tmp_path, monkeypatch):
    monkeypatch.setattr(
        EpubCheckRunner, "check", lambda *args: EpubCheckResult(False, "unavailable")
    )
    source = _write_book(tmp_path, {
        "index.xhtml": '<p>Image <img src="../../outside.png" alt="Fallback"/></p>',
    })
    output_dir = tmp_path / "out"
    with pytest.raises(InvalidEpubError, match="unsafe-link-target"):
        optimize_epub(source, output_dir)
    assert not list(output_dir.glob("*.epub"))
