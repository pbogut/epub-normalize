use crate::xml::Node;
use crate::{
    archive,
    uri::{Uri, decode},
    xml::Xml,
};
use anyhow::Result;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub fn is_content(item: &Node) -> bool {
    matches!(
        item.get("media-type").to_lowercase().as_str(),
        "application/xhtml+xml" | "text/html" | "application/x-dtbook+xml"
    ) && !item.get("href").is_empty()
}
pub fn html(path: &str) -> bool {
    matches!(
        archive::extension(Path::new(path)).as_str(),
        "html" | "htm" | "xhtml"
    )
}
pub fn anchors(root: &Node) -> BTreeSet<String> {
    root.all()
        .iter()
        .flat_map(|n| {
            ["id", "name"]
                .into_iter()
                .filter(|a| n.has(a))
                .map(|a| n.get(a))
        })
        .collect()
}
pub fn fragment_exists(path: &Path, fragment: &str) -> bool {
    Xml::read(path, true).is_ok_and(|xml| anchors(&xml.root).contains(fragment))
}

pub fn hyperlinks(work: &Path, package_dir: &str, items: &[Node]) -> Result<Vec<String>> {
    let mut documents = Vec::new();
    let mut by_path = BTreeMap::new();
    let mut by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut by_anchor: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for item in items.iter().filter(|i| is_content(i)) {
        let uri = Uri::parse(&item.get("href"));
        if uri.external || uri.path.is_empty() {
            continue;
        }
        let Ok(path) = archive::join(package_dir, &decode(&uri.path)) else {
            continue;
        };
        if !work.join(&path).is_file() || by_path.contains_key(&path) {
            continue;
        }
        let xml = Xml::read(&work.join(&path), true)?;
        let ids = anchors(&xml.root);
        for id in &ids {
            by_anchor
                .entry(id.clone())
                .or_default()
                .insert(path.clone());
        }
        by_name
            .entry(archive::basename(&path).into())
            .or_default()
            .insert(path.clone());
        by_path.insert(path.clone(), ids);
        documents.push((path, xml));
    }
    let mut actions = Vec::new();
    for (source, xml) in documents {
        let mut changed = false;
        for n in xml.root.named("a").iter().filter(|n| n.has("href")) {
            let href = n.get("href");
            let uri = Uri::parse(&href);
            if uri.external {
                continue;
            }
            let path = decode(&uri.path);
            let fragment = decode(&uri.fragment);
            let target = if path.is_empty() {
                Some(source.clone())
            } else {
                archive::join(archive::dirname(&source), &path).ok()
            };
            if let Some(target) = &target
                && work.join(target).is_file()
            {
                if fragment.is_empty() || (!by_path.contains_key(target) && !html(target)) {
                    continue;
                }
                if by_path
                    .get(target)
                    .is_some_and(|ids| ids.contains(&fragment))
                    || (!by_path.contains_key(target)
                        && fragment_exists(&work.join(target), &fragment))
                {
                    continue;
                }
            }
            let mut candidates = by_name
                .get(archive::basename(&path))
                .cloned()
                .unwrap_or_default();
            if !fragment.is_empty() {
                let matching = by_anchor.get(&fragment).cloned().unwrap_or_default();
                let common: BTreeSet<_> = candidates.intersection(&matching).cloned().collect();
                candidates = if common.is_empty() { matching } else { common };
            }
            if candidates.len() == 1 {
                let replacement =
                    uri.with_path(&archive::relative(&source, candidates.first().unwrap()));
                n.set("href", &replacement);
                actions.push(format!(
                    "Repaired hyperlink in {source}: {href} -> {replacement}"
                ));
            } else {
                n.del("href");
                actions.push(format!(
                    "Removed broken hyperlink in {source} (text preserved): {href}"
                ));
            }
            changed = true;
        }
        if changed {
            xml.write(&work.join(source))?;
        }
    }
    Ok(actions)
}
