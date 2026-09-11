use crate::{
    archive, repair,
    uri::{Uri, decode},
    xml::{Node, Xml},
};
use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::LazyLock,
};

pub const CSS_ID: &str = "epub-optimizer-css";
pub const CSS_HREF: &str = "Styles/epub-optimizer.css";
pub const CANONICAL_CSS: &str = include_str!("canonical.css");
const REMOVABLE_MEDIA: &str = "application/font-woff application/font-woff2 application/vnd.adobe-page-template+xml application/vnd.ms-opentype application/x-font-opentype application/x-font-otf application/x-font-ttf application/x-font-truetype application/x-font-woff font/otf font/sfnt font/ttf font/woff font/woff2";
#[derive(Debug, Serialize)]
pub struct Preview {
    pub input_filename: String,
    pub epub_version: Option<String>,
    pub package_path: String,
    pub content_documents: usize,
    pub stylesheets_and_fonts: usize,
    pub removable_files: usize,
    pub images_preserved: usize,
    pub would_write_canonical_css: bool,
    pub image_diagnostics: Vec<String>,
    pub change_summary: Vec<String>,
    pub warnings: Vec<String>,
}
pub fn package_path(work: &Path) -> Result<String> {
    let bytes = fs::read(work.join("META-INF/container.xml"))?;
    if String::from_utf8_lossy(&bytes).contains("<!ENTITY") {
        bail!("EntitiesForbidden in META-INF/container.xml.");
    }
    let xml = Xml::parse(&bytes, false)
        .map_err(|_| anyhow!("Could not parse META-INF/container.xml."))?;
    let files = xml.root.named("rootfile");
    let rootfile = files
        .iter()
        .find(|n| n.namespace() == "urn:oasis:names:tc:opendocument:xmlns:container")
        .or_else(|| files.iter().find(|n| n.namespace().is_empty()))
        .ok_or_else(|| anyhow!("container.xml does not declare an OPF rootfile."))?;
    let path = rootfile.get("full-path");
    if path.is_empty() {
        bail!("OPF rootfile path is empty.");
    }
    archive::entry_name(&path)?;
    if !work.join(&path).is_file() {
        bail!("Declared OPF package document does not exist.");
    }
    Ok(path)
}
pub fn manifest_items(manifest: &Node) -> Vec<Node> {
    manifest
        .children()
        .into_iter()
        .filter(|n| n.name() == "item")
        .collect()
}
fn optional_attr(n: &Node, attr: &str) -> Option<String> {
    n.has(attr).then(|| n.get(attr))
}
fn css_item(item: &Node) -> bool {
    item.get("media-type").eq_ignore_ascii_case("text/css")
        || archive::extension(Path::new(&item.get("href"))) == "css"
}
fn removable_item(item: &Node) -> bool {
    css_item(item)
        || crate::roles::one(&item.get("media-type").to_lowercase(), REMOVABLE_MEDIA)
        || crate::roles::one(
            &archive::extension(Path::new(&item.get("href"))),
            "css xpgt otf ttf woff woff2",
        )
}
static CSS_URLS: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?is)url\(\s*(?:'([^']*)'|"([^"]*)"|([^)]*?))\s*\)"#).unwrap()
});
fn resource_urls(text: &str) -> Vec<String> {
    CSS_URLS
        .captures_iter(text)
        .filter_map(|c| {
            let raw = c.get(1).or(c.get(2)).or(c.get(3))?.as_str().trim();
            if raw.is_empty()
                || raw.starts_with('/')
                || raw.starts_with("data:")
                || raw.split('/').next().unwrap_or("").contains(':')
            {
                return None;
            }
            let path = raw.split(['?', '#']).next().unwrap_or("").trim();
            (!path.is_empty()).then(|| path.to_string())
        })
        .collect()
}
pub fn removable_hrefs(
    work: &Path,
    dir: &str,
    items: &[Node],
    preserve: bool,
) -> Result<Vec<String>> {
    let mut fonts = BTreeSet::new();
    if preserve {
        let manifest_fonts: BTreeSet<_> = items
            .iter()
            .filter(|i| {
                !i.get("href").is_empty()
                    && (crate::roles::one(&i.get("media-type").to_lowercase(), REMOVABLE_MEDIA)
                        || crate::roles::one(
                            &archive::extension(Path::new(&i.get("href"))),
                            "css xpgt otf ttf woff woff2",
                        ))
            })
            .map(|i| i.get("href"))
            .collect();
        for item in items
            .iter()
            .filter(|i| css_item(i) && !i.get("href").is_empty())
        {
            let href = item.get("href");
            let path = work.join(archive::join(dir, &href)?);
            if !path.is_file() {
                continue;
            }
            let data = fs::read(path)?;
            for raw in resource_urls(&String::from_utf8_lossy(&data)) {
                if let Ok(target) = archive::join(archive::dirname(&href), &raw)
                    && manifest_fonts.contains(&target)
                {
                    fonts.insert(target);
                }
            }
        }
        for entry in walkdir::WalkDir::new(work) {
            let entry = entry?;
            if !entry.file_type().is_file() || !repair::html(&entry.path().to_string_lossy()) {
                continue;
            }
            let path = entry
                .path()
                .strip_prefix(work)?
                .to_string_lossy()
                .replace('\\', "/");
            for raw in resource_urls(&String::from_utf8_lossy(&fs::read(entry.path())?)) {
                if let Ok(target) = archive::join(archive::dirname(&path), &raw) {
                    let target = archive::relative(&format!("{dir}/package.opf"), &target);
                    if manifest_fonts.contains(&target) {
                        fonts.insert(target);
                    }
                }
            }
        }
    }
    Ok(items
        .iter()
        .filter(|i| {
            let href = i.get("href");
            !href.is_empty()
                && i.get("id") != CSS_ID
                && href != CSS_HREF
                && !(preserve && (css_item(i) || fonts.contains(&href)))
                && removable_item(i)
        })
        .map(|i| i.get("href"))
        .collect())
}
pub fn preview(input: &Path, preserve: bool) -> Result<Preview> {
    archive::validate(input)?;
    let temp = tempfile::Builder::new()
        .prefix("epub-optimizer-preview-")
        .tempdir()?;
    archive::extract(input, temp.path())?;
    let work = temp.path();
    let package = package_path(work)?;
    let dir = archive::dirname(&package);
    let xml = Xml::read(&work.join(&package), false)?;
    let manifest = xml
        .root
        .child("manifest")
        .ok_or_else(|| anyhow!("OPF package document is missing a manifest."))?;
    let items = manifest_items(&manifest);
    let removable = removable_hrefs(work, dir, &items, preserve)?;
    let content: Vec<_> = items.iter().filter(|i| repair::is_content(i)).collect();
    let images = items
        .iter()
        .filter(|i| i.get("media-type").to_lowercase().starts_with("image/"))
        .count();
    let mut warnings = Vec::new();
    for i in &content {
        let href = i.get("href");
        if !work.join(archive::join(dir, &href)?).is_file() {
            warnings.push(format!("Manifest content document is missing: {href}"));
        }
    }
    let mut count = 0;
    for href in &removable {
        count += usize::from(work.join(archive::join(dir, href)?).is_file());
    }
    Ok(Preview {
        input_filename: input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        epub_version: optional_attr(&xml.root, "version"),
        content_documents: content.len(),
        stylesheets_and_fonts: removable.len(),
        removable_files: count,
        images_preserved: images,
        would_write_canonical_css: true,
        image_diagnostics: image_diagnostics(work, dir, &items)?,
        change_summary: vec![
            format!("Would normalize {} content document(s).", content.len()),
            format!(
                "Would replace {} stylesheet/font manifest item(s).",
                removable.len()
            ),
            format!("Would delete {count} old style/font file(s)."),
            format!("Would preserve {images} image resource(s) without recompression."),
            "Would write the canonical EPUB Optimizer stylesheet.".into(),
        ],
        warnings,
        package_path: package,
    })
}
pub fn image_diagnostics(work: &Path, dir: &str, items: &[Node]) -> Result<Vec<String>> {
    let mut diagnostics = Vec::new();
    for item in items {
        let media = item.get("media-type").to_lowercase();
        let href = item.get("href");
        if !media.starts_with("image/") || href.is_empty() {
            continue;
        }
        let path = work.join(archive::join(dir, &href)?);
        if !path.is_file() {
            diagnostics.push(format!("Image missing from manifest: {href}."));
            continue;
        }
        let data = fs::read(path)?;
        let size = data.len();
        let dimensions = image_dimensions(&data[..size.min(4096)])
            .map(|(w, h)| format!(", {w}x{h}"))
            .unwrap_or_default();
        let size = if size < 1024 {
            format!("{size} B")
        } else if size < 1024 * 1024 {
            format!("{:.1} KB", size as f64 / 1024.0)
        } else {
            format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
        };
        diagnostics.push(format!("{href} ({media}, {size}{dimensions})."));
    }
    Ok(diagnostics)
}
fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
        return Some((
            u32::from_be_bytes(data[16..20].try_into().unwrap()),
            u32::from_be_bytes(data[20..24].try_into().unwrap()),
        ));
    }
    if data.starts_with(b"\xff\xd8") {
        let mut index = 2;
        while index + 9 < data.len() {
            if data[index] != 0xff {
                index += 1;
                continue;
            }
            if matches!(
                data[index + 1],
                0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb
            ) {
                return Some((
                    u16::from_be_bytes(data[index + 7..index + 9].try_into().unwrap()) as u32,
                    u16::from_be_bytes(data[index + 5..index + 7].try_into().unwrap()) as u32,
                ));
            }
            let len = u16::from_be_bytes(data[index + 2..index + 4].try_into().unwrap()) as usize;
            if len < 2 {
                return None;
            }
            index += 2 + len;
        }
    }
    None
}

pub fn clean_encryption(work: &Path, removed: &BTreeSet<String>) -> Result<usize> {
    let path = work.join("META-INF/encryption.xml");
    if removed.is_empty() || !path.is_file() {
        return Ok(0);
    }
    let xml = Xml::read(&path, false)?;
    let mut count = 0;
    for n in xml
        .root
        .named("encrypteddata")
        .iter()
        .filter(|n| n.namespace() == "http://www.w3.org/2001/04/xmlenc#")
    {
        let matches = n
            .descendants()
            .iter()
            .filter(|r| {
                r.name() == "cipherreference"
                    && r.namespace() == "http://www.w3.org/2001/04/xmlenc#"
            })
            .any(|r| {
                let uri = Uri::parse(&r.get("URI"));
                let decoded = decode(&uri.path);
                !uri.external
                    && !uri.path.is_empty()
                    && !uri.path.starts_with('/')
                    && !decoded.contains('\\')
                    && !decoded.split('/').any(|p| p == "..")
                    && archive::join("", &decoded).is_ok_and(|p| removed.contains(&p))
            });
        if matches {
            n.remove_with_tail();
            count += 1;
        }
    }
    if count > 0 {
        if xml.root.children().is_empty() {
            fs::remove_file(path)?;
        } else {
            xml.write(&path)?;
        }
    }
    Ok(count)
}

#[derive(Debug, Serialize)]
pub struct ValidationIssue {
    pub severity: String,
    pub code: String,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub input_filename: String,
    pub valid: bool,
    pub epub_version: Option<String>,
    pub package_path: String,
    pub issues: Vec<ValidationIssue>,
}
pub fn validate_details(input: &Path) -> Result<ValidationReport> {
    archive::validate(input)?;
    let temp = tempfile::tempdir()?;
    archive::extract(input, temp.path())?;
    let package = package_path(temp.path())?;
    let xml = Xml::read(&temp.path().join(&package), false)?;
    let issues = workspace_issues(temp.path(), archive::dirname(&package), &xml.root)?;
    Ok(ValidationReport {
        input_filename: input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        valid: !issues.iter().any(|i| i.severity == "error"),
        epub_version: optional_attr(&xml.root, "version"),
        package_path: package,
        issues,
    })
}
fn workspace_issues(work: &Path, dir: &str, package: &Node) -> Result<Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let mut add = |severity: &str, code: &str, message: String| {
        issues.push(ValidationIssue {
            severity: severity.into(),
            code: code.into(),
            message,
        })
    };
    let Some(manifest) = package.child("manifest") else {
        add(
            "error",
            "missing-manifest",
            "OPF package document is missing a manifest.".into(),
        );
        return Ok(issues);
    };
    let items = manifest_items(&manifest);
    let ids: BTreeSet<_> = items.iter().map(|i| i.get("id")).collect();
    for item in &items {
        let href = item.get("href");
        if !href.is_empty() && !work.join(archive::join(dir, &href)?).is_file() {
            add(
                "error",
                "missing-manifest-file",
                format!("Manifest file is missing: {href}"),
            );
        }
    }
    if let Some(spine) = package.child("spine") {
        for n in spine.children().iter().filter(|n| n.name() == "itemref") {
            let id = n.get("idref");
            if !ids.contains(&id) {
                add(
                    "error",
                    "missing-spine-target",
                    format!("Spine references missing manifest item: {id}"),
                );
            }
        }
    } else {
        add(
            "error",
            "missing-spine",
            "OPF package document is missing a spine.".into(),
        );
    }
    let mut duplicate_issues = Vec::new();
    for item in items.iter().filter(|i| repair::is_content(i)) {
        let href = item.get("href");
        let path = archive::join(dir, &href)?;
        if !work.join(&path).is_file() {
            continue;
        }
        let xml = Xml::read(&work.join(&path), true)?;
        let mut seen = BTreeSet::new();
        let mut duplicates = BTreeSet::new();
        for n in xml.root.all() {
            let id = n.get("id");
            if !id.is_empty() && !seen.insert(id.clone()) {
                duplicates.insert(id);
            }
            for attr in ["href", "src"] {
                let href = n.get(attr);
                if href.is_empty() {
                    continue;
                }
                let uri = Uri::parse(&href);
                if uri.dangerous() {
                    add(
                        "error",
                        "unsafe-link-scheme",
                        format!("Unsafe link scheme found: {href}"),
                    );
                    continue;
                }
                if uri.external || uri.path.is_empty() {
                    continue;
                }
                let Ok(target) = archive::join(archive::dirname(&path), &decode(&uri.path)) else {
                    add(
                        "error",
                        "unsafe-link-target",
                        format!("Link target escapes the EPUB root: {href}"),
                    );
                    continue;
                };
                if !work.join(&target).is_file() {
                    add(
                        "warning",
                        "missing-link-target",
                        format!("Link target is missing: {href}"),
                    );
                } else if !uri.fragment.is_empty()
                    && repair::html(&target)
                    && !repair::fragment_exists(&work.join(target), &decode(&uri.fragment))
                {
                    add(
                        "warning",
                        "missing-link-anchor",
                        format!("Link anchor is missing: {href}"),
                    );
                }
            }
        }
        for id in duplicates {
            duplicate_issues.push(ValidationIssue {
                severity: "warning".into(),
                code: "duplicate-id".into(),
                message: format!("Duplicate id '{id}' found in {href}."),
            });
        }
    }
    issues.extend(duplicate_issues);
    Ok(issues)
}
fn enforce_validation(report: &ValidationReport) -> Result<()> {
    let errors: Vec<_> = report
        .issues
        .iter()
        .filter(|i| i.severity == "error")
        .map(|i| format!("{}: {}", i.code, i.message))
        .collect();
    if !errors.is_empty() {
        bail!("Optimized EPUB failed validation: {}", errors.join("; "));
    }
    Ok(())
}

#[derive(Default)]
pub struct Options<'a> {
    pub output_filename: Option<&'a str>,
    pub preserve_publisher_css: bool,
    pub progress: Option<&'a dyn Fn(&str)>,
    pub checker: Option<&'a dyn crate::epubcheck::Checker>,
    pub max_size_bytes: Option<u64>,
}
#[derive(Debug)]
pub struct OptimizationResult {
    pub input_filename: String,
    pub output_path: PathBuf,
    pub output_filename: String,
    pub epub_version: Option<String>,
    pub package_path: String,
    pub elapsed_seconds: f64,
    pub content_documents_processed: usize,
    pub stylesheets_replaced: usize,
    pub images_preserved: usize,
    pub image_diagnostics: Vec<String>,
    pub repair_actions: Vec<String>,
    pub warnings: Vec<String>,
    pub log: Vec<String>,
    pub epubcheck: crate::epubcheck::Comparison,
    pub validation_outcome: String,
    pub validation_remaining: usize,
    pub validation_persisting: usize,
}
pub fn optimized_filename(filename: &str) -> String {
    let path = Path::new(filename);
    let mut stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    if stem.is_empty() {
        stem = "optimized".into();
    }
    if stem.to_lowercase().ends_with("-optimized") {
        stem.truncate(stem.len() - 10);
        if stem.is_empty() {
            stem = "optimized".into();
        }
    }
    let extension = if archive::extension(path) == "epub" {
        path.extension().unwrap().to_string_lossy().into_owned()
    } else {
        "epub".into()
    };
    format!("{stem}-optimized.{extension}")
}
fn enforce_comparison(comparison: &crate::epubcheck::Comparison) -> Result<()> {
    if !comparison.available || comparison.introduced.is_empty() {
        return Ok(());
    }
    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for f in &comparison.introduced {
        groups.entry((&f.code, &f.message)).or_default().push(f);
    }
    let details: Vec<_> = groups
        .into_iter()
        .map(|((code, message), findings)| {
            let paths: BTreeSet<_> = findings
                .iter()
                .filter_map(|f| f.path.as_deref())
                .filter(|p| !p.is_empty())
                .collect();
            format!(
                "{code}: {message} (x{}; resources: {})",
                findings.len(),
                paths.into_iter().collect::<Vec<_>>().join(", ")
            )
        })
        .collect();
    bail!(
        "optimized EPUB introduced EPUBCheck errors: {}",
        details.join("; ")
    )
}
fn outcome(
    comparison: &crate::epubcheck::Comparison,
    output: &crate::epubcheck::CheckResult,
) -> String {
    if !comparison.available {
        "unavailable"
    } else if output.errors().is_empty() {
        "clean"
    } else {
        "legacy_issues"
    }
    .into()
}
fn write_report(work: &Path, report: &serde_json::Value) -> Result<()> {
    let path = work.join("META-INF/epub-optimizer-report.json");
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(report)?))?;
    Ok(())
}
pub fn publish(staged: &Path, destination: &Path) -> Result<()> {
    use std::io::Write;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = tempfile::Builder::new()
        .prefix(".epub-optimizer-staging-")
        .tempdir_in(parent)?;
    let part = temp.path().join("publish.part");
    let mut output = fs::File::create(&part)?;
    std::io::copy(&mut fs::File::open(staged)?, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    crate::process::check_interrupt()?;
    fs::rename(part, destination)?;
    Ok(())
}

pub fn optimize(
    input: &Path,
    output_dir: &Path,
    options: &Options<'_>,
) -> Result<OptimizationResult> {
    use crate::{
        content,
        epubcheck::{self, Runner},
        navigation, process,
    };
    let start = std::time::Instant::now();
    let mut log = Vec::new();
    let mut warnings = Vec::new();
    let mut append_log = |message: String| {
        if let Some(progress) = options.progress {
            progress(&message);
        }
        log.push(message);
    };
    process::check_interrupt()?;
    fs::create_dir_all(output_dir)?;
    let input_filename = input
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let output_filename = options
        .output_filename
        .map(str::to_owned)
        .unwrap_or_else(|| optimized_filename(&input_filename));
    let output_path = output_dir.join(&output_filename);
    append_log("Validated EPUB archive.".into());
    archive::validate(input)?;
    if options
        .max_size_bytes
        .is_some_and(|max| fs::metadata(input).is_ok_and(|m| m.len() > max))
    {
        bail!("Input file exceeds the configured maximum upload size.");
    }
    let runner = Runner::from_env();
    let checker = options.checker.unwrap_or(&runner);
    let input_check = checker.check(&fs::canonicalize(input)?)?;
    append_log(format!(
        "EPUBCheck input status: {} ({} finding(s)).",
        input_check.status,
        input_check.findings.len()
    ));
    let temp = tempfile::Builder::new()
        .prefix("epub-optimizer-")
        .tempdir()?;
    let work = temp.path().join("work");
    let staged = temp.path().join("optimized.epub");
    archive::extract(input, &work)?;
    append_log("Extracted EPUB into a temporary workspace.".into());
    let package_path = package_path(&work)?;
    let dir = archive::dirname(&package_path);
    let package_file = work.join(&package_path);
    append_log(format!("Resolved OPF package document: {package_path}"));
    let package = Xml::read(&package_file, false)?;
    let epub_version = optional_attr(&package.root, "version");
    let manifest = package
        .root
        .child("manifest")
        .ok_or_else(|| anyhow!("OPF package document is missing a manifest."))?;
    let items = manifest_items(&manifest);
    let preserve = options.preserve_publisher_css;
    let removable = removable_hrefs(&work, dir, &items, preserve)?;
    let style_hrefs: Vec<_> = items
        .iter()
        .filter(|i| css_item(i) && !i.get("href").is_empty())
        .map(|i| i.get("href"))
        .collect();
    let class_roles = content::stylesheet_roles(&work, dir, &style_hrefs)?;
    let content_items: Vec<_> = items
        .iter()
        .filter(|i| repair::is_content(i))
        .cloned()
        .collect();
    let images = items
        .iter()
        .filter(|i| i.get("media-type").to_lowercase().starts_with("image/"))
        .count();
    let image_diagnostics = image_diagnostics(&work, dir, &items)?;
    let css = archive::join(dir, CSS_HREF)?;
    fs::create_dir_all(work.join(&css).parent().unwrap())?;
    fs::write(work.join(&css), CANONICAL_CSS)?;
    let mut canonical_exists = false;
    let mut replaced = 0;
    for item in &items {
        let href = item.get("href");
        if item.get("id") == CSS_ID || href == CSS_HREF {
            item.set("id", CSS_ID);
            item.set("href", CSS_HREF);
            item.set("media-type", "text/css");
            canonical_exists = true;
            continue;
        }
        if preserve && css_item(item) {
            continue;
        }
        if removable.contains(&href) && removable_item(item) {
            item.remove_with_tail();
            replaced += 1;
        }
    }
    if !canonical_exists {
        let item = manifest.like("item");
        item.set("id", CSS_ID);
        item.set("href", CSS_HREF);
        item.set("media-type", "text/css");
        manifest.append(item);
    }
    replaced = replaced.max(removable.len());
    let mut removed = BTreeSet::new();
    for href in &removable {
        let path = archive::join(dir, href)?;
        if work.join(&path).is_file() {
            fs::remove_file(work.join(&path))?;
            removed.insert(path);
        }
    }
    let encryption = clean_encryption(&work, &removed)?;
    append_log(format!(
        "Removed {replaced} old style/font manifest item(s)."
    ));
    if !removed.is_empty() {
        append_log(format!("Deleted {} old style/font file(s).", removed.len()));
    }
    if encryption > 0 {
        append_log(format!(
            "Removed {encryption} obsolete encryption record(s)."
        ));
    }
    let mut processed = 0;
    for item in &content_items {
        process::check_interrupt()?;
        let href = item.get("href");
        let path = archive::join(dir, &href)?;
        if !work.join(&path).is_file() {
            warnings.push(format!("Manifest content document is missing: {href}"));
            continue;
        }
        content::process(
            &work,
            &path,
            &css,
            &crate::roles::document(item),
            &class_roles,
            preserve,
        )?;
        processed += 1;
    }
    let normalized_nav = navigation::normalize(&work, dir, &items)?;
    if normalized_nav > 0 {
        append_log(format!(
            "Normalized {normalized_nav} navigation document(s)."
        ));
    }
    if navigation::ensure(&work, dir, &package.root, &manifest, &content_items)? {
        append_log("Generated missing EPUB 3 navigation document.".into());
    }
    append_log(format!("Processed {processed} content document(s)."));
    let metadata = if let Some(m) = package.root.child("metadata") {
        m
    } else {
        let m = package.root.like("metadata");
        if let Some(c) = package.root.nodes().first() {
            c.before(m.clone());
        } else {
            package.root.append(m.clone());
        }
        m
    };
    let marker = metadata
        .children()
        .into_iter()
        .find(|n| n.name() == "meta" && n.get("name") == "epub-optimizer:version")
        .unwrap_or_else(|| {
            let n = metadata.like("meta");
            n.set("name", "epub-optimizer:version");
            metadata.append(n.clone());
            n
        });
    marker.set("content", env!("CARGO_PKG_VERSION"));
    package.write(&package_file)?;
    let mut actions = repair::hyperlinks(&work, dir, &manifest_items(&manifest))?;
    for action in &actions {
        append_log(action.clone());
    }
    let mut report = serde_json::json!({ "generated_by": "EPUB Optimizer", "version": env!("CARGO_PKG_VERSION"), "input_filename": input_filename, "output_filename": output_filename, "epub_version": epub_version, "package_path": package_path, "content_documents_processed": processed, "stylesheets_replaced": replaced, "images_preserved": images, "image_diagnostics": image_diagnostics, "warnings": warnings, "repair_actions": actions });
    write_report(&work, &report)?;
    archive::write(&work, &staged)?;
    append_log("Repackaged optimized EPUB.".into());
    let validation = validate_details(&staged)?;
    warnings.extend(
        validation
            .issues
            .iter()
            .filter(|i| i.severity != "error")
            .map(|i| i.message.clone()),
    );
    append_log("Validated optimized EPUB output.".into());
    let mut output_check = checker.check(&staged)?;
    let mut repair_attempted = false;
    for _ in 0..3 {
        process::check_interrupt()?;
        if !output_check.available || output_check.errors().is_empty() {
            break;
        }
        let pass = repair::workspace(&work, &package_path, &package.root, &output_check.errors())?;
        if pass.is_empty() {
            break;
        }
        actions.extend(pass);
        repair_attempted = true;
        package.write(&package_file)?;
        report["repair_actions"] = serde_json::json!(actions);
        report["warnings"] = serde_json::json!(warnings);
        write_report(&work, &report)?;
        archive::write(&work, &staged)?;
        output_check = checker.check(&staged)?;
    }
    enforce_validation(&validate_details(&staged)?)?;
    if input_check.available && !output_check.available {
        bail!(
            "EPUBCheck unavailable during output validation{}; output not published",
            if repair_attempted {
                " after repair"
            } else {
                ""
            }
        );
    }
    if !input_check.available && output_check.available && !output_check.errors().is_empty() {
        bail!("EPUBCheck output errors cannot be classified without an input baseline");
    }
    let comparison = epubcheck::compare(&input_check, &output_check);
    enforce_comparison(&comparison)?;
    let validation_outcome = outcome(&comparison, &output_check);
    let remaining = output_check.error_count;
    let persisting = comparison.persisting.len();
    report["repair_actions"] = serde_json::json!(actions);
    report["warnings"] = serde_json::json!(warnings);
    report["validation_outcome"] = serde_json::json!(validation_outcome);
    report["validation_remaining"] = serde_json::json!(remaining);
    report["validation_persisting"] = serde_json::json!(persisting);
    write_report(&work, &report)?;
    archive::write(&work, &staged)?;
    let final_check = checker.check(&staged)?;
    if input_check.available && !final_check.available {
        bail!("EPUBCheck unavailable during final output validation; output not published");
    }
    if !input_check.available && final_check.available && !final_check.errors().is_empty() {
        bail!("EPUBCheck final output errors cannot be classified without an input baseline");
    }
    let comparison = epubcheck::compare(&input_check, &final_check);
    enforce_comparison(&comparison)?;
    if (
        outcome(&comparison, &final_check),
        final_check.error_count,
        comparison.persisting.len(),
    ) != (validation_outcome.clone(), remaining, persisting)
    {
        bail!(
            "EPUBCheck result changed after embedding the optimization report; output not published"
        );
    }
    append_log(format!(
        "EPUBCheck output status: {} ({} finding(s)).",
        final_check.status,
        final_check.findings.len()
    ));
    publish(&staged, &output_path)?;
    let elapsed = start.elapsed().as_secs_f64();
    append_log(format!("Finished in {elapsed:.2} seconds."));
    Ok(OptimizationResult {
        input_filename,
        output_path,
        output_filename,
        epub_version,
        package_path,
        elapsed_seconds: elapsed,
        content_documents_processed: processed,
        stylesheets_replaced: replaced,
        images_preserved: images,
        image_diagnostics,
        repair_actions: actions,
        warnings,
        log,
        epubcheck: comparison,
        validation_outcome,
        validation_remaining: remaining,
        validation_persisting: persisting,
    })
}
