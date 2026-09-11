use crate::{archive, xml::Node};

pub const FRONT: &str = "ack acknowledg afterword alsoby appendix ata author authorsnote bibliography colophon contents cop copyright ded dedication epigraph foreword glossary intro introduction notes preface prologue source toc title";
const BLOCK_ROLES: &str = "eo-blockquote eo-body eo-caption eo-centered eo-chapter eo-dedication eo-extract eo-first eo-footnote eo-front eo-front-body eo-front-list-item eo-front-section eo-hanging eo-image eo-letter eo-letter-attribution eo-letter-body eo-letter-first eo-letter-opener eo-list eo-metadata-line eo-metadata-page eo-metadata-title eo-part eo-poetry eo-right eo-scene-break eo-section eo-title-author eo-title-credit eo-title-credit-label eo-title-main eo-title-page eo-title-publisher eo-toc eo-toc-chapter eo-toc-entry eo-toc-heading eo-toc-part";
pub const INLINE_ROLES: &str = "eo-overline eo-smallcaps eo-strike eo-underline";
pub fn one(value: &str, words: &str) -> bool {
    words.split_whitespace().any(|w| w == value)
}
pub fn has(classes: &[String], words: &str) -> bool {
    classes.iter().any(|c| one(c, words))
}
pub fn existing(classes: &[String], words: &str) -> Option<String> {
    classes.iter().filter(|c| one(c, words)).min().cloned()
}
fn length(text: &str) -> usize {
    text.chars().count()
}
pub fn short_heading(text: &str) -> bool {
    length(text) <= 120 && text.split_whitespace().count() <= 14
}
fn short_list(text: &str) -> bool {
    length(text) <= 90 && text.split_whitespace().count() <= 12
}
pub fn image(n: &Node) -> bool {
    n.children().iter().any(|c| one(&c.name(), "img svg"))
}
fn descendant_image(n: &Node) -> bool {
    n.descendants()
        .iter()
        .any(|c| one(&c.name(), "img svg image"))
}
pub fn empty(n: &Node) -> bool {
    n.normalized().is_empty() && n.nodes().iter().all(Node::is_text)
}
pub fn scene(n: &Node) -> bool {
    matches!(
        n.normalized().as_str(),
        "*" | "* * *" | "***" | "****" | "*****"
    )
}
pub fn block_children(n: &Node) -> bool {
    n.children().iter().any(|c| {
        one(
            &c.name(),
            "blockquote div figure h1 h2 h3 h4 h5 h6 ol p section table ul",
        )
    })
}
fn link(n: &Node) -> bool {
    n.descendants().iter().any(|c| c.name() == "a")
}
pub fn all_upper(text: &str) -> bool {
    let letters: Vec<_> = text.chars().filter(|c| c.is_alphabetic()).collect();
    !letters.is_empty()
        && letters
            .iter()
            .all(|c| c.to_uppercase().to_string() == c.to_string())
}
fn category(text: &str) -> bool {
    (1..=4).contains(&text.split_whitespace().count()) && length(text) <= 60 && all_upper(text)
}
fn works_heading(text: &str) -> bool {
    [
        "also by",
        "also from",
        "by the same author",
        "other books by",
        "other works by",
        "works by",
    ]
    .iter()
    .any(|s| text.contains(s))
}
fn metadata_line(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "base",
        "book design",
        "cover art",
        "copyright",
        "digital",
        "edited",
        "editor",
        "edition",
        "epub",
        "isbn",
        "original title",
        "published",
        "publisher",
        "release",
        "rights",
        "title:",
        "version",
    ]
    .iter()
    .any(|s| lower.contains(s))
        || (text.contains(':') && length(text) <= 100)
        || (length(text) <= 40
            && text.chars().any(|c| c.is_numeric())
            && text.contains(['-', '/', '.', ':']))
}
fn numbered(token: &str, prefix: &str) -> bool {
    token
        .strip_prefix(prefix)
        .and_then(|s| s.chars().next())
        .is_some_and(|c| c.is_numeric())
}
fn chapterish(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["chapter ", "chap ", "kapitel ", "part ", "del "]
        .iter()
        .any(|s| lower.starts_with(s))
        || one(
            &lower,
            "afterword appendix epilogue introduction preface prolog prologue",
        )
}
pub fn document(item: &Node) -> String {
    let properties = item.get("properties").to_lowercase();
    if one("nav", &properties) {
        return "toc".into();
    }
    let values = format!("{} {} {properties}", item.get("id"), item.get("href")).to_lowercase();
    let normalized: String = values
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let tokens: Vec<_> = normalized.split_whitespace().map(str::to_owned).collect();
    let role = if has(&tokens, "cover coverpage titlepage")
        || (has(&tokens, "title") && !has(&tokens, "subtitle entitle"))
    {
        "title"
    } else if has(&tokens, "colophon credits creditos info") {
        "metadata"
    } else if has(&tokens, "prolog prologue") {
        "prologue"
    } else if has(&tokens, "intro introduction introduktion") {
        "introduction"
    } else if has(&tokens, "afterword appendix epilogue") {
        "chapter"
    } else if has(&tokens, "toc contents innehall innehåll") {
        "toc"
    } else if has(&tokens, FRONT) || tokens.iter().any(|t| t.starts_with("acknowledg")) {
        "front"
    } else if has(&tokens, "part del")
        || tokens
            .iter()
            .any(|t| numbered(t, "part") || numbered(t, "del"))
    {
        "part"
    } else if has(&tokens, "chapter chap kapitel")
        || tokens.iter().any(|t| {
            ["chapter", "chap", "kapitel"]
                .iter()
                .any(|p| numbered(t, p))
        })
        || values.contains("/ch")
    {
        "chapter"
    } else {
        "body"
    };
    role.into()
}
fn body_blocks(root: &Node) -> Vec<Node> {
    root.named("body")
        .into_iter()
        .flat_map(|b| b.descendants())
        .filter(|n| one(&n.name(), "div h1 h2 h3 p") && !n.normalized().is_empty())
        .collect()
}
pub fn refine(root: &Node, role: &str) -> String {
    if one(role, "part title toc works metadata") {
        return role.into();
    }
    let cover: Vec<_> = root
        .named("body")
        .into_iter()
        .flat_map(|b| b.descendants())
        .filter(|n| {
            one(&n.name(), "div figure p svg")
                && (!n.normalized().is_empty() || descendant_image(n))
        })
        .collect();
    if !cover.is_empty()
        && cover.len() <= 4
        && cover.iter().all(descendant_image)
        && length(
            &cover
                .iter()
                .map(Node::normalized)
                .collect::<Vec<_>>()
                .join(" "),
        ) <= 80
    {
        return "title".into();
    }
    let all = body_blocks(root);
    let leaf: Vec<_> = all.iter().filter(|n| !block_children(n)).collect();
    if leaf.len() >= 5
        && matches!(
            leaf[0].normalized().to_lowercase().as_str(),
            "contents" | "innehall" | "innehåll" | "table of contents"
        )
    {
        let leaf: Vec<_> = leaf.iter().take(80).collect();
        if leaf.iter().filter(|n| short_list(&n.normalized())).count() as f64 / leaf.len() as f64
            >= 0.8
            && (leaf.iter().filter(|n| link(n)).count() >= 3
                || leaf.iter().filter(|n| chapterish(&n.normalized())).count() >= 3)
        {
            return "toc".into();
        }
    }
    let dedication: Vec<_> = root
        .named("body")
        .into_iter()
        .flat_map(|b| b.descendants())
        .filter(|n| one(&n.name(), "blockquote div h1 h2 h3 p") && !n.normalized().is_empty())
        .collect();
    if (2..=12).contains(&dedication.len())
        && matches!(
            dedication[0].normalized().to_lowercase().as_str(),
            "dedication" | "dedicated to"
        )
        && dedication
            .iter()
            .filter(|n| {
                length(&n.normalized()) <= 120 && n.normalized().split_whitespace().count() <= 16
            })
            .count() as f64
            / dedication.len() as f64
            >= 0.75
    {
        return "dedication".into();
    }
    if (2..=14).contains(&leaf.len()) {
        let texts: Vec<_> = leaf.iter().map(|n| n.normalized()).collect();
        let hits = texts.iter().filter(|t| metadata_line(t)).count();
        let ratio = texts.iter().filter(|t| length(t) <= 90).count() as f64 / texts.len() as f64;
        let combined = texts.join(" ").to_lowercase();
        if (hits >= 2 && ratio >= 0.5)
            || (hits >= 1
                && ratio >= 0.75
                && ["epub", "isbn", "copyright", "editor"]
                    .iter()
                    .any(|s| combined.contains(s)))
        {
            return "metadata".into();
        }
    }
    if one(role, "prologue introduction") {
        if all.len() >= 3
            && (all[0].normalized().to_lowercase().contains(role)
                || (role == "introduction" && all[0].normalized().to_lowercase().contains("intro")))
        {
            let texts: Vec<_> = all
                .iter()
                .skip(1)
                .filter(|n| one(&n.name(), "div p"))
                .map(Node::normalized)
                .collect();
            let long = texts.iter().filter(|t| length(t) > 140).count();
            if texts.len() >= 2
                && long >= 2
                && long > texts.iter().filter(|t| short_list(t)).count()
            {
                return "body".into();
            }
        }
        return "front".into();
    }
    if leaf.len() >= 4
        && works_heading(
            &leaf
                .iter()
                .take(4)
                .map(|n| n.normalized().to_lowercase())
                .collect::<Vec<_>>()
                .join(" "),
        )
    {
        let sample: Vec<_> = leaf.iter().take(80).collect();
        if sample
            .iter()
            .filter(|n| short_list(&n.normalized()))
            .count() as f64
            / sample.len() as f64
            >= 0.65
            && (sample
                .iter()
                .filter(|n| n.descendants().iter().any(|c| one(&c.name(), "em i")))
                .count()
                >= 3
                || sample.iter().any(|n| category(&n.normalized())))
        {
            return "works".into();
        }
    }
    role.into()
}

pub fn boundary(role: &str) -> bool {
    one(
        role,
        "eo-caption eo-centered eo-chapter eo-dedication eo-extract eo-footnote eo-front eo-front-list-item eo-front-section eo-image eo-letter-attribution eo-metadata-line eo-metadata-title eo-part eo-poetry eo-right eo-scene-break eo-section eo-title-author eo-title-main eo-title-publisher eo-toc-chapter eo-toc-entry eo-toc-heading eo-toc-part",
    )
}
fn letter(c: &[String]) -> Option<&'static str> {
    if has(c, "ltg letter-greeting salutation") {
        Some("eo-letter-opener")
    } else if has(c, "ltf letter-first") {
        Some("eo-letter-first")
    } else if has(c, "lt ltl letter letter-body letter-last") {
        Some("eo-letter-body")
    } else {
        None
    }
}
fn letter_attribution(c: &[String]) -> bool {
    has(c, "ept letter-source letter-credit missive-source")
}
fn ancestor_class(n: &Node, class: &str) -> bool {
    let mut p = n.parent();
    while let Some(node) = p {
        if node.get("class").split_whitespace().any(|c| c == class) {
            return true;
        }
        p = node.parent();
    }
    false
}
fn tagged_ratio(n: &Node, tags: &str) -> f64 {
    let total = length(&n.normalized());
    if total == 0 {
        return 0.0;
    }
    (n.descendants()
        .iter()
        .filter(|c| one(&c.name(), tags))
        .map(|c| length(&c.normalized()))
        .sum::<usize>() as f64
        / total as f64)
        .min(1.0)
}
fn epigraph(n: &Node) -> bool {
    (20..=900).contains(&length(&n.normalized())) && !image(n) && tagged_ratio(n, "em i") >= 0.65
}
fn attribution(n: &Node) -> bool {
    let text = n.normalized();
    !text.is_empty()
        && length(&text) <= 140
        && (text.starts_with(['-', '—', '–'])
            || has(&n.classes(), "attribution source credit")
            || (tagged_ratio(n, "strong b") >= 0.6 && text.split_whitespace().count() <= 12))
}
fn first_meaningful(n: &Node) -> bool {
    n.parent()
        .and_then(|p| {
            p.children().into_iter().find(|c| {
                !one(&c.name(), "script style") && (!c.normalized().is_empty() || image(c))
            })
        })
        .is_some_and(|c| c == *n)
}
fn front_list(n: &Node) -> bool {
    if first_meaningful(n) {
        return false;
    }
    let Some(p) = n.parent() else {
        return false;
    };
    let siblings: Vec<_> = p
        .children()
        .into_iter()
        .filter(|c| !c.normalized().is_empty() || image(c))
        .collect();
    let texts: Vec<_> = siblings
        .iter()
        .filter(|c| !c.normalized().is_empty())
        .collect();
    siblings.len() >= 3
        && texts.len() >= 3
        && texts
            .iter()
            .filter(|c| !block_children(c) && short_list(&c.normalized()))
            .count() as f64
            / texts.len() as f64
            >= 0.75
}
fn heading(local: &str, c: &[String], front: bool) -> &'static str {
    if has(c, "eo-part part") {
        "eo-part"
    } else if has(c, "eo-front") || front || has(c, FRONT) {
        "eo-front"
    } else if has(c, "eo-section") {
        "eo-section"
    } else if has(c, "eo-chapter") || c.iter().any(|s| s.starts_with("chapter")) {
        "eo-chapter"
    } else if has(c, "section") || local != "h1" {
        "eo-section"
    } else {
        "eo-chapter"
    }
}
fn paragraph(n: &Node, c: &[String], after: bool, front: bool) -> &'static str {
    if image(n) {
        return "eo-image";
    }
    if scene(n) {
        return "eo-scene-break";
    }
    if let Some(role) = letter(c) {
        return role;
    }
    if letter_attribution(c) {
        return "eo-letter-attribution";
    }
    for (classes, role) in [
        ("caption figcaption", "eo-caption"),
        ("center center0 bl_center eo-centered", "eo-centered"),
        ("right bl_right attribution eo-right", "eo-right"),
        ("poem poetry verse line stanza", "eo-poetry"),
        ("footnote note endnote", "eo-footnote"),
        ("hanging reference bl_hanging d_hanging", "eo-hanging"),
        (
            "extract extract1 bl_extract bl_nonindent bl_indent stanga",
            "eo-extract",
        ),
    ] {
        if has(c, classes) {
            return role;
        }
    }
    if has(c, "nonindent nonindent1") || after {
        "eo-first"
    } else if front {
        "eo-front-body"
    } else {
        "eo-body"
    }
}
fn container(c: &[String]) -> Option<&'static str> {
    for (classes, role) in [
        ("part", "eo-part"),
        ("eo-right", "eo-right"),
        ("eo-centered", "eo-centered"),
        ("cover titlepage dis_img", "eo-image"),
        ("letter missive diary journal", "eo-letter"),
        ("block textbox abstract epigraph", "eo-extract"),
        ("poem poetry verse stanza", "eo-poetry"),
        ("hanging dialogue", "eo-hanging"),
        ("footnote note endnote", "eo-footnote"),
        ("dedication", "eo-dedication"),
        ("copyright otherbooks titlepage", "eo-centered"),
    ] {
        if has(c, classes) {
            return Some(role);
        }
    }
    None
}
fn dedication(n: &Node, after: bool) -> &'static str {
    if image(n) {
        "eo-image"
    } else if scene(n) {
        "eo-scene-break"
    } else if after
        && matches!(
            n.normalized().to_lowercase().as_str(),
            "dedication" | "dedicated to"
        )
    {
        "eo-front"
    } else {
        "eo-dedication"
    }
}
fn part(n: &Node, after: bool) -> &'static str {
    if image(n) {
        "eo-image"
    } else if scene(n) {
        "eo-scene-break"
    } else if after && short_heading(&n.normalized()) {
        "eo-part"
    } else if after {
        "eo-first"
    } else {
        "eo-body"
    }
}
fn credit_label(text: &str) -> bool {
    length(text) <= 80
        && (text.contains(" by")
            || text.contains("by ")
            || text.contains(" from ")
            || [
                "adapted ",
                "afterword ",
                "edited ",
                "illustrated ",
                "introduction ",
                "translated ",
                "with ",
            ]
            .iter()
            .any(|s| text.starts_with(s)))
}
fn title_line(n: &Node, index: usize) -> &'static str {
    if image(n) {
        return "eo-image";
    }
    if scene(n) {
        return "eo-scene-break";
    }
    let text = n.normalized();
    let lower = text.to_lowercase();
    if text.is_empty() {
        return "eo-scene-break";
    }
    if index == 0 {
        return "eo-title-main";
    }
    if credit_label(&lower) {
        return "eo-title-credit-label";
    }
    let earlier_credit = n.parent().is_some_and(|p| {
        p.children()
            .iter()
            .take_while(|c| *c != n)
            .any(|c| credit_label(&c.normalized().to_lowercase()))
    });
    if earlier_credit && index <= 2 {
        return "eo-title-credit";
    }
    let words = text.replace('/', " ");
    let words: Vec<_> = words.split_whitespace().collect();
    if [
        "books",
        "classics",
        "edition",
        "editions",
        "house",
        "imprint",
        "press",
        "publishers",
        "publishing",
    ]
    .iter()
    .any(|s| lower.contains(s))
        || (index >= 4
            && (1..=4).contains(&words.len())
            && words
                .iter()
                .all(|w| w.chars().next().is_some_and(char::is_uppercase)))
    {
        return "eo-title-publisher";
    }
    let words: Vec<_> = text
        .split_whitespace()
        .map(|s| s.trim_matches(['.', ',', ';', ':']))
        .collect();
    let alpha: Vec<_> = words
        .iter()
        .filter(|w| w.chars().any(char::is_alphabetic))
        .collect();
    if (1..=5).contains(&words.len())
        && !alpha.is_empty()
        && alpha
            .iter()
            .all(|w| w.chars().next().is_some_and(char::is_uppercase))
    {
        return "eo-title-author";
    }
    if index <= 2 {
        "eo-title-credit"
    } else {
        "eo-title-author"
    }
}
fn metadata(n: &Node, index: usize) -> &'static str {
    if image(n) {
        "eo-image"
    } else if scene(n) || empty(n) {
        "eo-scene-break"
    } else if (index == 0 && !metadata_line(&n.normalized())) || one(&n.name(), "h1 h2 h3") {
        "eo-metadata-title"
    } else {
        "eo-metadata-line"
    }
}
fn works(n: &Node, after: bool) -> &'static str {
    if image(n) {
        "eo-image"
    } else if scene(n) || empty(n) {
        "eo-scene-break"
    } else if after && works_heading(&n.normalized().to_lowercase()) {
        "eo-front"
    } else if category(&n.normalized()) {
        "eo-front-section"
    } else {
        "eo-front-list-item"
    }
}
fn toc(n: &Node, c: &[String], after: bool, is_block: bool) -> &'static str {
    if is_block && has(c, "toc toc_fm toc_bm eo-toc") && block_children(n) {
        return "eo-toc";
    }
    let text = n.normalized();
    if image(n) {
        return "eo-image";
    }
    if text.is_empty() {
        return "eo-scene-break";
    }
    if text
        .chars()
        .all(|c| c.is_whitespace() || ".*-_•·…".contains(c))
    {
        return "eo-toc-entry";
    }
    if after && short_heading(&text) && !link(n) {
        return "eo-toc-heading";
    }
    let letters: String = text.chars().filter(|c| c.is_alphabetic()).collect();
    let toc_part = link(n)
        && (["part ", "del "]
            .iter()
            .any(|s| text.to_lowercase().starts_with(s))
            || (text.split_whitespace().count() <= 4
                && (letters.is_empty() || all_upper(&letters))));
    if has(c, "toc_part eo-toc-part") || toc_part {
        return "eo-toc-part";
    }
    let chapter_link = n.descendants().iter().filter(|c| c.name() == "a").any(|a| {
        let href = a.get("href");
        let name = archive::basename(href.split('#').next().unwrap_or("")).to_lowercase();
        name.contains("chap")
            || numbered(&name, "c")
            || name
                .match_indices(['_', '-', '.'])
                .any(|(i, _)| numbered(&name[i + 1..], "c"))
    });
    if has(c, "toc_chap toc_sub eo-toc-chapter") || chapter_link || chapterish(&text) {
        "eo-toc-chapter"
    } else {
        "eo-toc-entry"
    }
}
fn anonymous_div(n: &Node, after: bool, role: &str, nested: bool) -> Option<&'static str> {
    if image(n) {
        return Some("eo-image");
    }
    if scene(n) || empty(n) {
        return Some("eo-scene-break");
    }
    if block_children(n) {
        return if role == "front" {
            Some("eo-front-body")
        } else {
            None
        };
    }
    let text = n.normalized();
    if text.is_empty() {
        return None;
    }
    if after && first_meaningful(n) && short_heading(&text) {
        return Some(if role == "front" {
            "eo-front"
        } else if role == "part" {
            "eo-part"
        } else {
            "eo-chapter"
        });
    }
    if role == "front" {
        return Some(if front_list(n) {
            "eo-front-list-item"
        } else {
            "eo-front-body"
        });
    }
    if role == "part" || after || nested {
        Some("eo-first")
    } else {
        Some("eo-body")
    }
}

pub fn classify(root: &Node, document_role: &str) {
    let mut after = true;
    let front = one(document_role, "dedication front title works");
    let mut title_index = 0;
    let mut metadata_index = 0;
    let mut opening = false;
    for n in root.named("body").into_iter().flat_map(|b| b.descendants()) {
        let local = n.name();
        let c = n.classes();
        let old = existing(&c, BLOCK_ROLES);
        if one(&local, "h1 h2 h3 h4 h5 h6") {
            if let Some(role) = old {
                let reclassify = one(&role, "eo-centered eo-right")
                    && !one(document_role, "dedication front metadata title toc works")
                    && (local == "h1"
                        || has(&c, "eo-chapter")
                        || c.iter().any(|c| c.starts_with("chapter")));
                if !reclassify {
                    n.set("class", &role);
                    after = boundary(&role);
                    opening = false;
                    continue;
                }
            }
            let role = match document_role {
                "toc" => "eo-toc-heading",
                "title" => {
                    let r = title_line(&n, title_index);
                    title_index += 1;
                    r
                }
                "works" => "eo-front",
                "metadata" => {
                    let r = metadata(&n, metadata_index);
                    metadata_index += 1;
                    r
                }
                _ => heading(&local, &c, front),
            };
            n.set("class", role);
            after = document_role != "title" || one(role, "eo-title-main eo-title-author");
            if !one(document_role, "title toc") {
                opening = false;
            }
            continue;
        }
        if let Some(role) = old
            && one(
                &local,
                "aside blockquote div figcaption figure ol p section ul",
            )
        {
            n.set("class", &role);
            after = boundary(&role);
            opening = one(&role, "eo-extract eo-scene-break");
            continue;
        }
        if one(&local, "ol ul blockquote figcaption aside") {
            let role = match local.as_str() {
                "ol" | "ul" => "eo-list",
                "figcaption" => "eo-caption",
                "aside" => "eo-footnote",
                _ if document_role == "dedication" => "eo-dedication",
                _ if has(&c, "eo-right") => "eo-right",
                _ if has(&c, "eo-centered") => "eo-centered",
                _ => "eo-blockquote",
            };
            n.set("class", role);
            after = true;
            continue;
        }
        if local == "p" {
            if letter(&c).is_some() || ancestor_class(&n, "eo-letter") {
                n.set("class", letter(&c).unwrap_or("eo-letter-body"));
                after = false;
                continue;
            }
            if letter_attribution(&c) {
                n.set("class", "eo-letter-attribution");
                after = true;
                continue;
            }
            let special = match document_role {
                "toc" => Some((
                    toc(&n, &c, after, false),
                    "eo-image eo-toc-heading eo-toc-part",
                    "eo-toc-heading",
                )),
                "part" => Some((
                    part(&n, after),
                    "eo-image eo-part eo-scene-break",
                    "eo-part",
                )),
                "title" => {
                    let r = title_line(&n, title_index);
                    title_index += 1;
                    Some((r, "eo-image eo-title-main eo-title-author", "eo-title-main"))
                }
                "works" => Some((
                    works(&n, after),
                    "eo-front eo-front-section eo-image",
                    "eo-front",
                )),
                "metadata" => {
                    let r = metadata(&n, metadata_index);
                    metadata_index += 1;
                    Some((r, "*", ""))
                }
                "dedication" => Some((dedication(&n, after), "*", "eo-front")),
                _ => None,
            };
            if let Some((role, boundaries, rename)) = special {
                if role == rename {
                    n.rename("h1");
                }
                n.set("class", role);
                after = boundaries == "*" || one(role, boundaries);
                continue;
            }
            if after && !front && epigraph(&n) {
                n.set("class", "eo-extract");
                after = true;
                opening = true;
                continue;
            }
            if opening && attribution(&n) {
                n.set("class", "eo-extract");
                after = true;
                opening = false;
                continue;
            }
            let role = paragraph(&n, &c, after, front);
            n.set("class", role);
            after = one(
                role,
                "eo-caption eo-centered eo-extract eo-footnote eo-front-list-item eo-image eo-letter-attribution eo-poetry eo-scene-break",
            );
            opening = one(role, "eo-extract eo-scene-break");
            continue;
        }
        let direct = n.parent().is_some_and(|p| p.name() == "body");
        if local == "div" && direct {
            let role = match document_role {
                "toc" => Some(toc(&n, &c, after, true)),
                "title" => Some(if block_children(&n) {
                    "eo-title-page"
                } else {
                    let r = title_line(&n, title_index);
                    title_index += 1;
                    r
                }),
                "metadata" => Some(if block_children(&n) {
                    "eo-metadata-page"
                } else {
                    let r = metadata(&n, metadata_index);
                    metadata_index += 1;
                    r
                }),
                "works" => Some(if block_children(&n) {
                    "eo-front-body"
                } else {
                    works(&n, after)
                }),
                "dedication" => Some(dedication(&n, after)),
                _ => container(&c).or_else(|| anonymous_div(&n, after, document_role, false)),
            };
            if let Some(role) = role {
                if one(
                    role,
                    "eo-chapter eo-front eo-part eo-section eo-title-main eo-toc-heading",
                ) {
                    n.rename("h1");
                    n.set("class", role);
                    after = true;
                    opening = false;
                } else if role == "eo-image" || block_children(&n) {
                    n.set("class", role);
                    after = true;
                    opening = false;
                } else {
                    n.rename("p");
                    n.set("class", role);
                    after = one(
                        role,
                        "eo-caption eo-centered eo-extract eo-footnote eo-front-list-item eo-front-section eo-image eo-metadata-line eo-metadata-title eo-poetry eo-scene-break eo-title-author eo-title-credit eo-title-credit-label eo-title-publisher eo-toc-entry eo-toc-chapter eo-toc-part",
                    );
                    opening = one(role, "eo-extract eo-scene-break");
                }
                continue;
            }
        }
        if local == "div" && one(document_role, "front works metadata title toc") {
            let role = match document_role {
                "front" => container(&c).or_else(|| anonymous_div(&n, after, "front", true)),
                "works" => Some(if block_children(&n) {
                    "eo-front-body"
                } else {
                    works(&n, after)
                }),
                "metadata" => Some(if block_children(&n) {
                    "eo-metadata-page"
                } else {
                    let r = metadata(&n, metadata_index);
                    metadata_index += 1;
                    r
                }),
                "title" => Some(if block_children(&n) {
                    "eo-title-page"
                } else {
                    let r = title_line(&n, title_index);
                    title_index += 1;
                    r
                }),
                _ => Some(toc(&n, &c, after, true)),
            };
            if let Some(role) = role {
                if one(role, "eo-front eo-title-main eo-toc-heading") {
                    n.rename("h1");
                    n.set("class", role);
                    after = true;
                } else if block_children(&n) || one(role, "eo-title-page eo-metadata-page eo-toc") {
                    n.set("class", role);
                    after = true;
                } else {
                    n.rename("p");
                    n.set("class", role);
                    after = match document_role {
                        "front" => one(role, "eo-front-list-item eo-scene-break eo-image"),
                        "works" => one(role, "eo-front eo-front-section eo-image eo-scene-break"),
                        "metadata" => true,
                        "title" => one(
                            role,
                            "eo-image eo-title-author eo-title-main eo-title-publisher eo-scene-break",
                        ),
                        _ => one(
                            role,
                            "eo-scene-break eo-toc-chapter eo-toc-entry eo-toc-part",
                        ),
                    };
                }
                continue;
            }
        }
        if one(&local, "div figure section")
            && let Some(role) = container(&c)
        {
            n.set("class", role);
            after = one(
                role,
                "eo-dedication eo-extract eo-footnote eo-image eo-poetry eo-toc",
            );
        }
        if local == "img" {
            if let Some(p) = n.parent()
                && one(&p.name(), "div figure p")
            {
                p.add_class("eo-image");
            }
            after = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn document_names_use_tokens_not_substrings() {
        let item = Node::element("item", "");
        item.set("href", "Man_som_hatar_kvinnor_split_1.html");
        assert_eq!(document(&item), "body");
        item.set("href", "appendix.xhtml");
        assert_eq!(document(&item), "chapter");
        item.set("properties", "nav");
        assert_eq!(document(&item), "toc");
    }
}
