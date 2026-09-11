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
