use epub_normalize::{
    archive, content, navigation, repair, roles,
    xml::{Node, Xml},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};

fn fingerprint(n: &Node) -> Value {
    if n.is_text() {
        return json!(n.normalized());
    }
    if !n.is_element() {
        return json!({"raw": n.serialize()});
    }
    let attrs: BTreeMap<_, _> = n
        .attrs()
        .into_iter()
        .filter(|(k, _)| k != "xmlns" && !k.starts_with("xmlns:"))
        .collect();
    json!({"name": n.name(), "namespace": n.namespace(), "attrs": attrs,
        "children": n.nodes().iter().filter(|c| !c.is_text() || !c.normalized().is_empty()).map(fingerprint).collect::<Vec<_>>()})
}

#[test]
fn content_matches_python_fixtures() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/compatibility.json")).unwrap();
    for case in cases {
        if case.get("error").is_some() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let preserve = case["preserve"].as_bool().unwrap();
        let temp = tempfile::tempdir().unwrap();
        archive::extract(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.epub")),
            temp.path(),
        )
        .unwrap();
        let package_path = case["preview"]["package_path"].as_str().unwrap();
        let dir = archive::dirname(package_path);
        let package = Xml::read(&temp.path().join(package_path), false).unwrap();
        let manifest = package.root.child("manifest").unwrap();
        let items = manifest.children();
        let hrefs = items
            .iter()
            .filter(|i| i.get("media-type") == "text/css")
            .map(|i| i.get("href"))
            .collect::<Vec<_>>();
        let class_roles = content::stylesheet_roles(temp.path(), dir, &hrefs).unwrap();
        for item in items.iter().filter(|i| repair::is_content(i)) {
            let path = archive::join(dir, &item.get("href")).unwrap();
            content::process(
                temp.path(),
                &path,
                &archive::join(dir, "Styles/epub-optimizer.css").unwrap(),
                &roles::document(item),
                &class_roles,
                preserve,
            )
            .unwrap();
        }
        navigation::normalize(temp.path(), dir, &items).unwrap();
        repair::hyperlinks(temp.path(), dir, &items).unwrap();
        for item in items
            .iter()
            .filter(|i| repair::is_content(i) || i.get("media-type") == "application/x-dtbncx+xml")
        {
            let path = archive::join(dir, &item.get("href")).unwrap();
            let expected: Vec<u8> = serde_json::from_value(case["output"][&path].clone()).unwrap();
            let expected = Xml::parse(&expected, false).unwrap();
            let actual = Xml::read(&temp.path().join(&path), false).unwrap();
            assert_eq!(
                fingerprint(&actual.root),
                fingerprint(&expected.root),
                "{name}, preserve={preserve}, {path}"
            );
        }
    }
}

#[test]
fn previews_match_python_fixtures() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/compatibility.json")).unwrap();
    for case in cases {
        if case.get("error").is_some() {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let preserve = case["preserve"].as_bool().unwrap();
        let input =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.epub"));
        let preview = epub_normalize::core::preview(&input, preserve).unwrap();
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            case["preview"],
            "{name}, preserve={preserve}"
        );
    }
}

#[test]
fn complete_archives_match_python_fixtures() {
    use epub_normalize::{
        core::{self, Options},
        epubcheck::Runner,
    };
    use std::io::Read;
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/compatibility.json")).unwrap();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let preserve = case["preserve"].as_bool().unwrap();
        let input =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.epub"));
        let output = tempfile::tempdir().unwrap();
        let checker = Runner::default();
        let options = Options {
            output_filename: Some("normalized.epub"),
            preserve_publisher_css: preserve,
            checker: Some(&checker),
            ..Default::default()
        };
        let result = core::optimize(&input, output.path(), &options);
        if let Some(error) = case.get("error") {
            assert_eq!(result.unwrap_err().to_string(), error.as_str().unwrap());
            continue;
        }
        let result = result.unwrap_or_else(|e| panic!("{name}, preserve={preserve}: {e}"));
        let mut archive =
            zip::ZipArchive::new(std::fs::File::open(&result.output_path).unwrap()).unwrap();
        let expected = case["output"].as_object().unwrap();
        assert_eq!(archive.len(), expected.len(), "{name}");
        assert_eq!(archive.by_index(0).unwrap().name(), "mimetype");
        assert_eq!(
            archive.by_index(0).unwrap().compression(),
            zip::CompressionMethod::Stored
        );
        for (path, bytes) in expected {
            let expected: Vec<u8> = serde_json::from_value(bytes.clone()).unwrap();
            let mut actual = Vec::new();
            archive
                .by_name(path)
                .unwrap()
                .read_to_end(&mut actual)
                .unwrap();
            if path.ends_with(".json") {
                assert_eq!(
                    serde_json::from_slice::<Value>(&actual).unwrap(),
                    serde_json::from_slice::<Value>(&expected).unwrap(),
                    "{name}, preserve={preserve}, {path}"
                );
            } else if [".xml", ".opf", ".xhtml", ".html", ".ncx"]
                .iter()
                .any(|suffix| path.ends_with(suffix))
            {
                let expected = Xml::parse(&expected, false).unwrap();
                let actual = Xml::parse(&actual, false).unwrap();
                assert_eq!(
                    fingerprint(&actual.root),
                    fingerprint(&expected.root),
                    "{name}, preserve={preserve}, {path}"
                );
            } else {
                assert_eq!(actual, expected, "{name}, preserve={preserve}, {path}");
            }
        }
        let first_bytes = std::fs::read(&result.output_path).unwrap();
        let second = core::optimize(&input, output.path(), &options).unwrap();
        assert_eq!(
            first_bytes,
            std::fs::read(second.output_path).unwrap(),
            "non-deterministic archive for {name}"
        );
    }
}
