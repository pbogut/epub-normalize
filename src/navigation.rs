use crate::{
    archive,
    roles::one,
    uri::{Uri, decode},
    xml::{Node, Xml, escape, escape_text},
};
use anyhow::Result;
use std::{collections::HashMap, fs, path::Path};

pub fn normalize(work: &Path, dir: &str, items: &[Node]) -> Result<usize> {
    let mut count = 0;
    for item in items {
        if item.get("media-type").to_lowercase() != "application/x-dtbncx+xml"
            || item.get("href").is_empty()
        {
            continue;
        }
        let path = archive::join(dir, &item.get("href"))?;
        let file = work.join(&path);
        if !file.is_file() {
            continue;
        }
        let xml = Xml::read(&file, true)?;
        let mut changed = false;
        for n in xml.root.named("navpoint") {
            let src = n.child("content").map(|c| c.get("src")).unwrap_or_default();
            let uri = Uri::parse(&src);
            let missing = !uri.external
                && (uri.path.is_empty()
                    || archive::join(archive::dirname(&path), &decode(&uri.path))
                        .map_or(true, |p| !work.join(p).is_file()));
            if src.is_empty() || uri.dangerous() || missing {
                n.remove_with_tail();
                changed = true;
                continue;
            }
            if let Some(label) = n.child("navlabel").and_then(|l| l.child("text")) {
                let text = label.normalized();
                if label.content() != text {
                    label.set_text(&text);
                    changed = true;
                }
            }
        }
        let mut orders = HashMap::new();
        let mut next = 1;
        for (i, n) in xml
            .root
            .all()
            .iter()
            .filter(|n| one(&n.name(), "navpoint navtarget pagetarget"))
            .enumerate()
        {
            let src = n.child("content").map(|c| c.get("src")).unwrap_or_default();
            let key = if src.is_empty() {
                format!("missing:{i}")
            } else {
                format!("src:{src}")
            };
            let order = orders.entry(key).or_insert_with(|| {
                let order = next.to_string();
                next += 1;
                order
            });
            if n.get("playOrder") != *order {
                n.set("playOrder", order);
                changed = true;
            }
        }
        if changed {
            xml.write(&file)?;
            count += 1;
        }
    }
    Ok(count)
}

pub fn ensure(
    work: &Path,
    dir: &str,
    package: &Node,
    manifest: &Node,
    content: &[Node],
) -> Result<bool> {
    if !package.get("version").starts_with('3')
        || content
            .iter()
            .any(|i| one("nav", &i.get("properties").to_lowercase()))
    {
        return Ok(false);
    }
    let Some(spine) = package.child("spine") else {
        return Ok(false);
    };
    let items: Vec<_> = spine
        .children()
        .iter()
        .filter(|r| r.name() == "itemref")
        .filter_map(|r| content.iter().find(|i| i.get("id") == r.get("idref")))
        .collect();
    if items.is_empty() {
        return Ok(false);
    }
    let href = if work.join(archive::join(dir, "nav.xhtml")?).exists() {
        "epub-optimizer-nav.xhtml"
    } else {
        "nav.xhtml"
    };
    let nav = archive::join(dir, href)?;
    let mut entries = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let path = archive::join(dir, &item.get("href"))?;
        let mut label = format!("Section {}", index + 1);
        if work.join(&path).is_file() {
            let xml = Xml::read(&work.join(&path), true)?;
            let heading = xml.root.all().into_iter().find(|n| one(&n.name(), "h1 h2"));
            let title = xml.root.named("title").into_iter().next();
            if let Some(node) = heading
                .filter(|n| !n.normalized().is_empty())
                .or_else(|| title.filter(|n| !n.normalized().is_empty()))
            {
                label = node.normalized();
            }
        }
        entries.push(format!(
            "      <li><a href=\"{}\">{}</a></li>",
            escape(&archive::relative(&nav, &path)),
            escape_text(&label)
        ));
    }
    fs::write(
        work.join(nav),
        format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n  <head><title>Contents</title></head>\n  <body>\n    <nav epub:type=\"toc\" role=\"doc-toc\">\n      <h1>Contents</h1>\n      <ol>\n{}\n      </ol>\n    </nav>\n  </body>\n</html>\n",
            entries.join("\n")
        ),
    )?;
    let item = manifest.like("item");
    item.set("id", "epub-optimizer-nav");
    item.set("href", href);
    item.set("media-type", "application/xhtml+xml");
    item.set("properties", "nav");
    manifest.append(item);
    Ok(true)
}
