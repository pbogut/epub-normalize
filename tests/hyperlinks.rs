use epub_normalize::{
    repair,
    xml::{Node, Xml},
};
use std::{fs, path::Path};

fn documents(root: &Path, contents: &[(&str, &str)]) -> Vec<Node> {
    contents.iter().map(|(name,body)| {
        let path = root.join(name); fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Test</title></head><body>{body}</body></html>")).unwrap();
        let item = Node::element("item", ""); item.set("href", name); item.set("media-type", "application/xhtml+xml"); item
    }).collect()
}
#[test]
fn unique_targets_repair_moved_split_and_encoded_links() {
    let cases = [
        (
            "przypisy.html",
            "../Text/chapter-05.html#note",
            vec![("OEBPS/Text/chapter-05.html", "<p id=\"note\">Text</p>")],
            "OEBPS/Text/chapter-05.html#note",
        ),
        (
            "Text/notes.xhtml",
            "../../old/book.html?q=1#note%20one",
            vec![(
                "Nowe części/rozdział 1.xhtml",
                "<a name=\"note one\">Text</a>",
            )],
            "../Nowe%20cz%C4%99%C5%9Bci/rozdzia%C5%82%201.xhtml?q=1#note%20one",
        ),
        (
            "index.xhtml",
            "chapter.xhtml#note",
            vec![
                ("chapter.xhtml", "<p id=\"other\">Chapter</p>"),
                ("unrelated.xhtml", "<p id=\"note\">Note</p>"),
            ],
            "unrelated.xhtml#note",
        ),
        (
            "notes.xhtml",
            "../old/chapter.html#note",
            vec![
                ("Text/chapter.html", "<p id=\"note\">Correct</p>"),
                ("Text/other.html", "<p id=\"note\">Other</p>"),
            ],
            "Text/chapter.html#note",
        ),
        (
            "notes.xhtml",
            "../old/chapter.html",
            vec![("Text/chapter.html", "<p>Text</p>")],
            "Text/chapter.html",
        ),
    ];
    for (source, href, mut targets, expected) in cases {
        let temp = tempfile::tempdir().unwrap();
        let body = format!("<p><a href=\"{href}\">Return</a></p>");
        targets.insert(0, (source, &body));
        let items = documents(temp.path(), &targets);
        let actions = repair::hyperlinks(temp.path(), "", &items).unwrap();
        assert_eq!(actions.len(), 1);
        let xml = Xml::read(&temp.path().join(source), false).unwrap();
        assert_eq!(xml.root.named("a")[0].get("href"), expected);
        assert!(
            repair::hyperlinks(temp.path(), "", &items)
                .unwrap()
                .is_empty()
        );
    }
}
#[test]
fn ambiguous_and_stale_links_lose_only_the_destination() {
    for href in [
        "/C:/Documents%20and%20Settings/heg/Pulpit/harry%20potter/#toc",
        "../old/chapter.xhtml#note",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let body =
            format!("<p>Before <a id=\"top-link\" href=\"{href}\"><em>[top]</em></a> after.</p>");
        let items = documents(
            temp.path(),
            &[
                ("index.xhtml", &body),
                ("one/chapter.xhtml", "<p id=\"note\">One</p>"),
                ("two/chapter.xhtml", "<p id=\"note\">Two</p>"),
            ],
        );
        let actions = repair::hyperlinks(temp.path(), "", &items).unwrap();
        assert_eq!(actions.len(), 1);
        let xml = Xml::read(&temp.path().join("index.xhtml"), false).unwrap();
        assert!(!xml.root.named("a")[0].has("href"));
        assert_eq!(xml.root.named("a")[0].get("id"), "top-link");
        assert_eq!(xml.root.named("p")[0].content(), "Before [top] after.");
        assert_eq!(xml.root.named("em")[0].content(), "[top]");
    }
}
#[test]
fn valid_links_and_resource_attributes_remain_byte_identical() {
    let temp = tempfile::tempdir().unwrap();
    let items = documents(
        temp.path(),
        &[
            (
                "Text/chapter.xhtml",
                "<p id=\"local\"><a href=\"#local\">Local</a><a href=\"../other.xhtml#note\">Other</a><a href=\"https://example.com/missing#note\">Web</a><a href=\"//example.com/\">Web</a><a href=\"mailto:a@b.com\">Mail</a><a href=\"\">Top</a><img src=\"../../outside.png\"/></p>",
            ),
            ("other.xhtml", "<p id=\"note\">Note</p>"),
        ],
    );
    let before = fs::read(temp.path().join("Text/chapter.xhtml")).unwrap();
    assert!(
        repair::hyperlinks(temp.path(), "", &items)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read(temp.path().join("Text/chapter.xhtml")).unwrap(),
        before
    );
}
#[test]
fn absolute_destinations_do_not_resolve_outside_the_archive() {
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.xhtml");
    fs::write(&outside, "<html><p id=\"note\">Private</p></html>").unwrap();
    let work = temp.path().join("work");
    let body = format!("<a href=\"{}#note\">Note</a>", outside.display());
    let items = documents(&work, &[("index.xhtml", &body)]);
    let actions = repair::hyperlinks(&work, "", &items).unwrap();
    assert!(actions[0].starts_with("Removed broken hyperlink"));
    assert!(
        !Xml::read(&work.join("index.xhtml"), false)
            .unwrap()
            .root
            .named("a")[0]
            .has("href")
    );
}
