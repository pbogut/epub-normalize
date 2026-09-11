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

fn normalized_path(path: &str) -> String {
    archive::join("", path.replace('\\', "/").trim_start_matches('/'))
        .unwrap_or_else(|_| path.into())
}

pub fn workspace(
    work: &Path,
    package_path: &str,
    package: &Node,
    findings: &[crate::epubcheck::Finding],
) -> Result<Vec<String>> {
    let codes: BTreeSet<_> = findings.iter().map(|f| f.code.to_uppercase()).collect();
    let references = codes.contains("RSC-007") || codes.contains("RSC-012");
    let missing_files = codes.contains("OPF-003");
    let missing_entries = codes.contains("OPF-012");
    let metadata_findings: Vec<_> = findings
        .iter()
        .filter(|f| {
            f.path
                .as_ref()
                .is_some_and(|p| normalized_path(&decode(p)) == normalized_path(package_path))
        })
        .cloned()
        .collect();
    let mut actions = repair_metadata(package, &metadata_findings);
    let Some(manifest) = package.child("manifest") else {
        return Ok(actions);
    };
    let dir = archive::dirname(package_path);
    let mut items: Vec<_> = manifest
        .children()
        .into_iter()
        .filter(|n| n.name() == "item")
        .collect();
    let ncx_paths: BTreeSet<_> = findings
        .iter()
        .filter(|f| f.code.to_uppercase() == "NCX-001")
        .filter_map(|f| f.path.as_ref())
        .map(|p| normalized_path(&decode(p)))
        .collect();
    if (codes.contains("OPF-030") && !actions.is_empty()) || !ncx_paths.is_empty() {
        let identifier = package
            .child("metadata")
            .and_then(|m| {
                m.children().into_iter().find(|n| {
                    n.name() == "identifier" && n.get("id") == package.get("unique-identifier")
                })
            })
            .map(|n| n.content().trim().to_string())
            .unwrap_or_default();
        if !identifier.is_empty() {
            for item in &items {
                if item.get("media-type").to_lowercase() != "application/x-dtbncx+xml" {
                    continue;
                }
                let href = item.get("href");
                let Ok(path) = archive::join(dir, &decode(&href)) else {
                    continue;
                };
                if !ncx_paths.is_empty() && !ncx_paths.contains(&normalized_path(&path)) {
                    continue;
                }
                let Ok(xml) = Xml::read(&work.join(path.clone()), false) else {
                    continue;
                };
                if let Some(n) = xml
                    .root
                    .named("meta")
                    .iter()
                    .find(|n| n.get("name").to_lowercase() == "dtb:uid")
                    && n.get("content") != identifier
                {
                    n.set("content", &identifier);
                    xml.write(&work.join(path))?;
                    actions.push(format!(
                        "Synchronized NCX identifier with OPF identifier: {href}"
                    ));
                }
            }
        }
    }
    if !references && !missing_files && !missing_entries {
        return Ok(actions);
    }
    let spine_ids: BTreeSet<_> = package
        .child("spine")
        .map(|s| s.children().iter().map(|n| n.get("idref")).collect())
        .unwrap_or_default();
    if missing_files {
        items.retain(|item| {
            let href = item.get("href");
            if !href.is_empty()
                && !spine_ids.contains(&item.get("id"))
                && archive::join(dir, &href).is_ok_and(|p| !work.join(p).is_file())
            {
                item.remove_with_tail();
                actions.push(format!("Removed missing manifest item: {href}"));
                false
            } else {
                true
            }
        });
    }
    let affected: BTreeSet<_> = findings
        .iter()
        .filter(|f| matches!(f.code.to_uppercase().as_str(), "RSC-007" | "RSC-012"))
        .filter_map(|f| f.path.as_ref())
        .map(|p| normalized_path(&decode(p)))
        .collect();
    let content: Vec<_> = items.iter().filter(|i| is_content(i)).cloned().collect();
    let mut ids: BTreeSet<_> = items.iter().map(|i| i.get("id")).collect();
    for item in content {
        let Ok(path) = archive::join(dir, &decode(&item.get("href"))) else {
            continue;
        };
        let fix_refs = references && affected.contains(&normalized_path(&path));
        if !fix_refs && !missing_entries {
            continue;
        }
        let Ok(xml) = Xml::read(&work.join(&path), true) else {
            continue;
        };
        let mut changed = false;
        for n in xml.root.all() {
            let attrs = n.attrs();
            let mut names: Vec<_> = ["href", "src", "data"]
                .into_iter()
                .filter(|a| attrs.contains_key(*a))
                .map(str::to_string)
                .collect();
            names.extend(
                attrs
                    .keys()
                    .filter(|a| a.contains(':') && a.rsplit(':').next() == Some("href"))
                    .cloned(),
            );
            for attr in names {
                let value = n.get(&attr);
                if value.is_empty() {
                    continue;
                }
                let uri = Uri::parse(&value);
                if uri.external {
                    continue;
                }
                if uri.path.is_empty() {
                    if !uri.fragment.is_empty()
                        && fix_refs
                        && !fragment_exists(&work.join(&path), &decode(&uri.fragment))
                    {
                        remove_reference(&n, &attr, &value, &mut actions);
                        changed = true;
                    }
                    continue;
                }
                let Ok(target) = archive::join(archive::dirname(&path), &decode(&uri.path)) else {
                    continue;
                };
                let exists = work.join(&target).is_file();
                if !uri.fragment.is_empty() && (!exists || html(&target)) {
                    if exists && fragment_exists(&work.join(&target), &decode(&uri.fragment)) {
                        continue;
                    }
                    if !exists {
                        if !fix_refs {
                            continue;
                        }
                        remove_reference(&n, &attr, &value, &mut actions);
                    } else {
                        n.set(&attr, &uri.path);
                        actions.push(format!("Removed broken fragment: {value}"));
                    }
                    changed = true;
                    continue;
                }
                if !exists {
                    if fix_refs {
                        remove_reference(&n, &attr, &value, &mut actions);
                        changed = true;
                    }
                    continue;
                }
                if missing_entries
                    && !items
                        .iter()
                        .any(|i| archive::join(dir, &i.get("href")).is_ok_and(|p| p == target))
                    && let Some(media) = mime_guess::from_path(&target).first_raw()
                {
                    let stem = Path::new(&target)
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy();
                    let base = format!(
                        "optimizer-{}",
                        stem.chars()
                            .map(|c| if c.is_alphanumeric() { c } else { '-' })
                            .collect::<String>()
                            .trim_matches('-')
                    );
                    let mut id = base.clone();
                    let mut index = 2;
                    while ids.contains(&id) {
                        id = format!("{base}-{index}");
                        index += 1;
                    }
                    let href = archive::relative(package_path, &target);
                    let item = manifest.like("item");
                    item.set("id", &id);
                    item.set("href", &href);
                    item.set("media-type", media);
                    manifest.append(item.clone());
                    items.push(item);
                    ids.insert(id);
                    actions.push(format!(
                        "Added manifest item for referenced resource: {href}"
                    ));
                    changed = true;
                }
            }
        }
        if changed {
            xml.write(&work.join(path))?;
        }
    }
    Ok(actions)
}

fn repair_metadata(package: &Node, findings: &[crate::epubcheck::Finding]) -> Vec<String> {
    let mut actions = Vec::new();
    let codes: BTreeSet<_> = findings.iter().map(|f| f.code.to_uppercase()).collect();
    let Some(metadata) = package.child("metadata") else {
        return actions;
    };
    if codes.contains("OPF-030") {
        let identifiers: Vec<_> = metadata
            .children()
            .into_iter()
            .filter(|n| n.name() == "identifier" && !n.content().trim().is_empty())
            .collect();
        if let Some(chosen) = identifiers
            .iter()
            .find(|n| !n.get("id").is_empty())
            .or(identifiers.first())
        {
            let mut id = chosen.get("id");
            if id.is_empty() {
                id = "bookid".into();
                let mut index = 2;
                while metadata.children().iter().any(|n| n.get("id") == id) {
                    id = format!("bookid-{index}");
                    index += 1;
                }
                chosen.set("id", &id);
                actions.push(format!("Assigned stable id to dc:identifier: {id}"));
            }
            package.set("unique-identifier", &id);
            actions.push(format!(
                "Pointed package unique-identifier to dc:identifier: {id}"
            ));
        }
    }
    if codes.contains("OPF-028") {
        let re = regex::Regex::new(r#"(?i)prefix:\s*"([A-Za-z][\w-]*)""#).unwrap();
        for f in findings
            .iter()
            .filter(|f| f.code.to_uppercase() == "OPF-028")
        {
            if let Some(c) = re.captures(&f.message) {
                let prefix = &c[1];
                let prefixes = package.get("prefix");
                if prefix.eq_ignore_ascii_case("calibre")
                    && !prefixes
                        .split_whitespace()
                        .step_by(2)
                        .any(|p| p.strip_suffix(':') == Some(prefix))
                {
                    package.set(
                        "prefix",
                        format!("{prefixes} calibre: https://calibre-ebook.com").trim(),
                    );
                    actions.push("Declared known calibre metadata vocabulary".into());
                }
            }
        }
    }
    if codes.contains("RSC-005") {
        let re = regex::Regex::new(r#"Property "([^"]+)" must refine"#).unwrap();
        let invalid: BTreeSet<_> = findings
            .iter()
            .filter_map(|f| re.captures(&f.message).map(|c| c[1].to_string()))
            .collect();
        for n in metadata
            .children()
            .iter()
            .filter(|n| n.name() == "meta" && n.has("refines"))
        {
            let reference = n.get("refines");
            let property = n.get("property");
            let target = metadata.children().into_iter().find(|n| {
                reference
                    .strip_prefix('#')
                    .is_some_and(|r| n.get("id") == r)
            });
            let allowed = match property.as_str() {
                "role" => "creator contributor publisher",
                "file-as" => "creator contributor",
                "title-type" => "title",
                _ => "",
            };
            if property.is_empty()
                || target.is_none()
                || (invalid.contains(&property)
                    && !allowed.is_empty()
                    && target
                        .as_ref()
                        .is_some_and(|t| !crate::roles::one(&t.name(), allowed)))
            {
                n.remove_with_tail();
                actions.push("Removed invalid metadata refinement".into());
            }
        }
    }
    actions
}
fn remove_reference(n: &Node, attr: &str, value: &str, actions: &mut Vec<String>) {
    let local = n.name();
    let message = if matches!(local.as_str(), "a" | "area") && attr == "href" {
        n.del(attr);
        "Removed broken link target (text preserved)"
    } else if local == "img" {
        let alt = n.get("alt").trim().to_string();
        if !alt.is_empty() {
            let span = n.like("span");
            span.set_text(&alt);
            n.before(span);
            n.detach();
            "Replaced broken image with alt text"
        } else {
            n.remove_with_tail();
            "Removed broken image without alt text"
        }
    } else if matches!(local.as_str(), "object" | "audio" | "video") {
        n.del(attr);
        "Removed broken resource attribute; fallback preserved"
    } else if matches!(local.as_str(), "link" | "script" | "image" | "source") {
        n.remove_with_tail();
        "Removed broken resource element"
    } else {
        n.del(attr);
        "Removed broken reference"
    };
    actions.push(format!("{message}: {value}"));
}
