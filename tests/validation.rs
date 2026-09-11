use epub_normalize::{
    archive,
    core::{self, Options},
    epubcheck::{self, CheckResult, Checker, Finding},
    repair,
    xml::Xml,
};
use serde_json::json;
use std::{cell::RefCell, collections::VecDeque, fs, path::Path};

struct Sequence(RefCell<VecDeque<CheckResult>>);
impl Checker for Sequence {
    fn check(&self, _: &Path) -> anyhow::Result<CheckResult> {
        Ok(self
            .0
            .borrow_mut()
            .pop_front()
            .expect("unexpected EPUBCheck invocation"))
    }
}
fn clean() -> CheckResult {
    epubcheck::parse_report("{\"messages\":[]}", 0)
}
fn finding(code: &str, path: Option<&str>, message: &str) -> CheckResult {
    epubcheck::parse_report(&json!({"messages": [{"ID": code, "severity": "error", "message": message, "locations": [{"path": path}]}]}).to_string(), 1)
}
fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.epub")
}

#[test]
fn validation_failures_never_replace_existing_output() {
    let missing = CheckResult::unavailable("unavailable", "no checker");
    let bad = finding("PKG-001", Some("content.opf"), "Bad archive");
    for (checks, expected) in [
        (
            vec![missing.clone(), bad.clone()],
            "without an input baseline",
        ),
        (vec![clean(), bad.clone()], "introduced EPUBCheck errors"),
        (
            vec![clean(), clean(), bad.clone()],
            "introduced EPUBCheck errors",
        ),
        (
            vec![bad.clone(), clean(), bad.clone()],
            "changed after embedding",
        ),
        (
            vec![clean(), missing.clone()],
            "unavailable during output validation",
        ),
        (
            vec![clean(), clean(), missing.clone()],
            "unavailable during final output validation",
        ),
        (
            vec![missing.clone(), missing.clone(), bad.clone()],
            "without an input baseline",
        ),
    ] {
        let out = tempfile::tempdir().unwrap();
        let output = out.path().join("normalized.epub");
        fs::write(&output, b"existing").unwrap();
        let checker = Sequence(RefCell::new(checks.into()));
        let options = Options {
            checker: Some(&checker),
            output_filename: Some("normalized.epub"),
            ..Default::default()
        };
        let error = core::optimize(&fixture(), out.path(), &options).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(fs::read(output).unwrap(), b"existing");
        assert_eq!(fs::read_dir(out.path()).unwrap().count(), 1);
    }
}

#[test]
fn existing_findings_remain_advisory() {
    let old = finding(
        "RSC-005",
        Some("OEBPS/Text/chapter.xhtml"),
        "Existing structure",
    );
    let checker = Sequence(RefCell::new(vec![old.clone(), old.clone(), old].into()));
    let out = tempfile::tempdir().unwrap();
    let result = core::optimize(
        &fixture(),
        out.path(),
        &Options {
            checker: Some(&checker),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.validation_outcome, "legacy_issues");
    assert_eq!(result.validation_remaining, 1);
    assert_eq!(result.validation_persisting, 1);
}

#[test]
fn checker_failure_after_resource_repair_keeps_output() {
    let temp = tempfile::tempdir().unwrap();
    let work = temp.path().join("work");
    archive::extract(&fixture(), &work).unwrap();
    let file = work.join("OEBPS/Text/chapter.xhtml");
    let text = fs::read_to_string(&file).unwrap().replace(
        "</body>",
        "<object data=\"missing.bin\">Fallback</object></body>",
    );
    fs::write(&file, text).unwrap();
    let input = temp.path().join("book.epub");
    archive::write(&work, &input).unwrap();
    let broken = finding(
        "RSC-007",
        Some("OEBPS/Text/chapter.xhtml"),
        "Missing resource",
    );
    let checker = Sequence(RefCell::new(
        vec![
            broken.clone(),
            broken,
            CheckResult::unavailable("timeout", "timeout"),
        ]
        .into(),
    ));
    let out = temp.path().join("out");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("book-optimized.epub"), b"existing").unwrap();
    let error = core::optimize(
        &input,
        &out,
        &Options {
            checker: Some(&checker),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unavailable during output validation after repair")
    );
    assert_eq!(
        fs::read(out.join("book-optimized.epub")).unwrap(),
        b"existing"
    );
}

fn package() -> Xml {
    Xml::parse(br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="chapter" href="Text/chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="chapter"/></spine></package>"#, false).unwrap()
}

#[test]
fn finding_gated_repairs_preserve_text_and_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("OEBPS/Text/chapter.xhtml");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    let original = "<html><body><p><a href=\"missing.xhtml#section\"><em>Text stays</em></a><img src=\"missing.jpg\" alt=\"City map\"/></p><object data=\"missing.bin\">Fallback</object></body></html>";
    fs::write(&file, original).unwrap();
    for (code, path) in [
        ("RSC-007", None),
        ("RSC-005", Some("OEBPS/Text/chapter.xhtml")),
    ] {
        let actions = repair::workspace(
            temp.path(),
            "OEBPS/package.opf",
            &package().root,
            &finding(code, path, "Missing").errors(),
        )
        .unwrap();
        assert!(actions.is_empty());
        assert_eq!(fs::read_to_string(&file).unwrap(), original);
    }
    let actions = repair::workspace(
        temp.path(),
        "OEBPS/package.opf",
        &package().root,
        &finding("RSC-007", Some("OEBPS/Text/chapter.xhtml"), "Missing").errors(),
    )
    .unwrap();
    assert_eq!(actions.len(), 3);
    let doc = Xml::read(&file, false).unwrap();
    assert_eq!(doc.root.normalized(), "Text staysCity mapFallback");
    assert!(!doc.root.named("a")[0].has("href"));
    assert!(!doc.root.named("object")[0].has("data"));
}

#[test]
fn metadata_and_ncx_repairs_preserve_primary_and_unknown_metadata() {
    let xml = Xml::parse(br##"<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="missing"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier>urn:isbn:123</dc:identifier><dc:title id="title">Title stays</dc:title><dc:creator id="author">Author stays</dc:creator><meta property="calibre:timestamp">2020-01-01</meta><meta property="vendor:accessibility">Preserve this</meta><meta refines="#missing" property="title-type">main</meta><meta refines="#title" property="role">aut</meta><meta refines="#author" property="role">aut</meta></metadata><manifest><item id="ncx" href="toc%20file.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine/></package>"##, false).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let ncx = temp.path().join("OEBPS/toc file.ncx");
    fs::create_dir_all(ncx.parent().unwrap()).unwrap();
    fs::write(
        &ncx,
        "<ncx><head><meta name=\"dtb:uid\" content=\"old\"/></head></ncx>",
    )
    .unwrap();
    let findings: Vec<Finding> = [
        ("OPF-028", "Undeclared prefix: \"calibre\"."),
        ("OPF-028", "Undeclared prefix: \"vendor\"."),
        ("OPF-030", "Missing identifier"),
        ("RSC-005", "Property \"role\" must refine a creator"),
    ]
    .iter()
    .flat_map(|(c, m)| finding(c, Some("OEBPS/book.opf"), m).errors())
    .collect();
    let actions = repair::workspace(temp.path(), "OEBPS/book.opf", &xml.root, &findings).unwrap();
    assert_eq!(xml.root.get("unique-identifier"), "bookid");
    assert!(xml.root.get("prefix").contains("calibre:"));
    assert_eq!(xml.root.named("title")[0].content(), "Title stays");
    assert!(
        xml.root
            .named("meta")
            .iter()
            .any(|n| n.get("property") == "vendor:accessibility")
    );
    assert_eq!(
        xml.root
            .named("meta")
            .iter()
            .filter(|n| n.has("refines"))
            .count(),
        1
    );
    assert!(fs::read_to_string(ncx).unwrap().contains("urn:isbn:123"));
    assert!(actions.iter().any(|a| a.starts_with("Synchronized NCX")));
}

#[test]
fn archive_collisions_and_unsafe_resource_paths() {
    use std::io::Write;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("case.epub");
    let mut zip = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, data) in [
        ("Text/Café.xhtml", "first"),
        ("text/cafe\u{301}.xhtml", "last"),
    ] {
        zip.start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(data.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let work = temp.path().join("work");
    archive::extract(&path, &work).unwrap();
    assert_eq!(
        fs::read_to_string(work.join("Text/Café.xhtml")).unwrap(),
        "last"
    );
    assert!(!work.join("text").exists());
    let work = temp.path().join("unsafe");
    archive::extract(&fixture(), &work).unwrap();
    let doc = work.join("OEBPS/Text/chapter.xhtml");
    let text = fs::read_to_string(&doc).unwrap().replace(
        "</body>",
        "<p><img src=\"../../../outside.png\"/></p></body>",
    );
    fs::write(doc, text).unwrap();
    archive::write(&work, &path).unwrap();
    let error = core::optimize(
        &path,
        &temp.path().join("out"),
        &Options {
            checker: Some(&epubcheck::Runner::default()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("unsafe-link-target"));
}
