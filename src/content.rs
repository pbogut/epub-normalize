use crate::{
    archive,
    roles::{self, has, one},
    uri::{Uri, decode},
    xml::{Node, Xml},
};
use anyhow::Result;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::LazyLock,
};

static RIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"text-align\s*:\s*right\b").unwrap());
static CENTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"text-align\s*:\s*center\b").unwrap());
static CSS_CLASSES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.([A-Za-z0-9_-]+)\s*\{([^}]*)\}").unwrap());
pub fn stylesheet_roles(
    root: &Path,
    package_dir: &str,
    hrefs: &[String],
) -> Result<HashMap<String, String>> {
    let mut roles = HashMap::new();
    for href in hrefs {
        if !href
            .to_lowercase()
            .split('#')
            .next()
            .unwrap_or("")
            .ends_with(".css")
        {
            continue;
        }
        let path = root.join(archive::join(package_dir, href)?);
        if !path.is_file() {
            continue;
        }
        let data = fs::read(path)?;
        let css = String::from_utf8_lossy(&data);
        for c in CSS_CLASSES.captures_iter(&css) {
            let declarations = c[2].to_lowercase();
            if RIGHT.is_match(&declarations) {
                roles.insert(c[1].to_lowercase(), "eo-right".into());
            } else if CENTER.is_match(&declarations) {
                roles.insert(c[1].to_lowercase(), "eo-centered".into());
            }
        }
    }
    Ok(roles)
}

pub fn process(
    root: &Path,
    path: &str,
    css: &str,
    document_role: &str,
    class_roles: &HashMap<String, String>,
    preserve: bool,
) -> Result<()> {
    let xml = Xml::read(&root.join(path), true)?;
    normalize(
        &xml.root,
        root,
        path,
        css,
        document_role,
        class_roles,
        preserve,
    );
    xml.write(&root.join(path))
}
pub fn normalize(
    root: &Node,
    work: &Path,
    path: &str,
    css: &str,
    document_role: &str,
    class_roles: &HashMap<String, String>,
    preserve: bool,
) {
    if let Some(head) = root.named("head").first() {
        for n in head.children().iter().filter(|n| n.name() == "link") {
            let href = n.get("href");
            let rel = n.get("rel").to_lowercase();
            let typ = n.get("type").to_lowercase();
            if href.ends_with("epub-optimizer.css")
                || rel == "xpgt"
                || typ.contains("page-template")
                || (rel.contains("stylesheet") && !preserve)
            {
                n.remove_with_tail();
            }
        }
        let link = head.like("link");
        link.set("href", &archive::relative(path, css));
        link.set("rel", "stylesheet");
        link.set("type", "text/css");
        head.append(link);
    }
    if !preserve {
        for n in root.named("style") {
            n.remove_with_tail();
        }
    }
    for n in root.all() {
        for attr in ["href", "src"] {
            let value = n.get(attr);
            let scheme = value.split_once(':').map_or("", |(s, _)| s);
            if one(&scheme.to_lowercase(), "javascript data vbscript file") {
                n.del(attr);
            }
        }
        let style = n.get("style").to_lowercase();
        let align = n.get("align").to_lowercase();
        let mut right = align == "right" || RIGHT.is_match(&style);
        let mut center = !right && (align == "center" || CENTER.is_match(&style));
        for c in n.classes() {
            if let Some(role) = class_roles.get(&c) {
                right |= role == "eo-right";
                center |= role == "eo-centered";
            }
        }
        if right {
            n.add_class("eo-right");
        } else if center {
            n.add_class("eo-centered");
        }
        for attr in n.attrs().keys() {
            let local = attr.rsplit(':').next().unwrap_or(attr).to_lowercase();
            if one(&n.name(), "svg image") && one(&local, "height width") {
                continue;
            }
            if one(
                &local,
                "align alink background bgcolor border cellpadding cellspacing clear color face height hspace link marginheight marginwidth size style text valign vlink vspace width",
            ) {
                n.del(attr);
            }
        }
    }
    for n in root.named("img") {
        let src = n.get("src");
        let uri = Uri::parse(&src);
        let missing = !uri.external
            && !uri.path.is_empty()
            && archive::join(archive::dirname(path), &decode(&uri.path))
                .is_ok_and(|p| !work.join(p).is_file());
        if src.is_empty() || missing {
            n.detach();
        }
    }
    for n in root.descendants() {
        if one(&n.name(), "p div figure")
            && has(&n.classes(), "eo-image image img dis_img cover")
            && roles::empty(&n)
        {
            n.detach();
        }
    }
    for n in root.named("font") {
        n.unwrap();
    }
    for n in root.named("span") {
        let c = n.classes();
        if let Some(role) = roles::existing(&c, roles::INLINE_ROLES) {
            n.set("class", &role);
        } else if has(&c, "bold") {
            n.rename("strong");
            n.del("class");
        } else if has(&c, "italic ital") {
            n.rename("em");
            n.del("class");
        } else if has(&c, "underline") {
            n.set("class", "eo-underline");
        } else if has(&c, "strike") {
            n.set("class", "eo-strike");
        } else if has(&c, "overline") {
            n.set("class", "eo-overline");
        } else if has(&c, "smallcaps small-cap small-caps")
            || (n.normalized().chars().count() >= 2 && roles::all_upper(&n.normalized()))
        {
            n.set("class", "eo-smallcaps");
        } else {
            n.del("class");
        }
    }
    wrap_phrasing(root);
    duplicate_ids(root);
    loop {
        let mut changed = false;
        for n in root.named("blockquote") {
            let children = n.children();
            if children.len() != 1
                || children[0].name() != "blockquote"
                || n.nodes()
                    .iter()
                    .filter(|c| c.is_text())
                    .any(|c| !c.content().trim().is_empty())
                || n.parent().is_none()
            {
                continue;
            }
            n.before(children[0].clone());
            n.detach();
            changed = true;
        }
        if !changed {
            break;
        }
    }
    for n in root.descendants() {
        if !one(&n.name(), "div p section")
            || !roles::empty(&n)
            || has(
                &n.classes(),
                "scene-break scenebreak separator ornament space-break",
            )
        {
            continue;
        }
        if n.parent()
            .is_some_and(|p| p.name() == "body" && p.children().len() == 1)
        {
            continue;
        }
        n.detach();
    }
    roles::classify(root, &roles::refine(root, document_role));
    for n in root.all() {
        if n.has("class") {
            let c = n.get("class");
            let retained: Vec<_> = c
                .split_whitespace()
                .filter(|c| c.starts_with("eo-"))
                .collect();
            if retained.is_empty() {
                n.del("class");
            } else {
                n.set("class", &retained.join(" "));
            }
        }
    }
}

fn wrap_phrasing(root: &Node) {
    for container in root
        .all()
        .into_iter()
        .filter(|n| one(&n.name(), "body blockquote"))
    {
        let mut run = Vec::new();
        for child in container.nodes() {
            if child.is_element()
                && one(
                    &child.name(),
                    "address blockquote del div dl h1 h2 h3 h4 h5 h6 hr ins noscript ol p pre script table ul svg",
                )
            {
                flush_run(&container, &mut run);
                continue;
            }
            run.push(child);
        }
        flush_run(&container, &mut run);
    }
}
fn flush_run(container: &Node, run: &mut Vec<Node>) {
    if run
        .iter()
        .any(|n| !n.is_text() || !n.content().trim().is_empty())
    {
        let p = container.like("p");
        if container.namespace().is_empty() {
            p.set("xmlns", "http://www.w3.org/1999/xhtml");
        }
        run[0].before(p.clone());
        for child in run.drain(..) {
            p.append(child);
        }
    }
    run.clear();
}
fn duplicate_ids(root: &Node) {
    let mut seen = HashSet::new();
    let mut counters = HashMap::new();
    let mut replacements = HashMap::new();
    for n in root.all() {
        let id = n.get("id");
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.clone()) {
            continue;
        }
        let count = counters.entry(id.clone()).or_insert(1);
        *count += 1;
        let mut new_id = format!("{id}-{count}");
        while seen.contains(&new_id) {
            *count += 1;
            new_id = format!("{id}-{count}");
        }
        n.set("id", &new_id);
        seen.insert(new_id.clone());
        replacements.entry(id).or_insert(new_id);
    }
    for n in root.all() {
        if let Some(fragment) = n.get("href").strip_prefix('#')
            && let Some(new_id) = replacements.get(fragment)
        {
            n.set("href", &format!("#{new_id}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phrasing_text_and_emphasis_survive_wrapping() {
        let xml = Xml::parse(b"<html><body>A<font>B<em>C</em>D</font>E<h1>Heading</h1><span class='italic'>After</span></body></html>", false).unwrap();
        normalize(
            &xml.root,
            Path::new("."),
            "chapter.xhtml",
            "Styles/epub-optimizer.css",
            "body",
            &HashMap::new(),
            false,
        );
        assert_eq!(xml.root.normalized(), "ABCDEHeadingAfter");
        assert_eq!(xml.root.named("p").len(), 2);
        assert_eq!(xml.root.named("em").len(), 2);
    }
}
