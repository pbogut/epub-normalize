use anyhow::{Result, bail};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    path::Path,
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

pub fn entry_name(name: &str) -> Result<String> {
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') {
        bail!("Archive entry uses an absolute path: {name}");
    }
    if normalized.split('/').next().unwrap_or("").contains(':') {
        bail!("Archive entry uses a drive-qualified path: {name}");
    }
    if normalized.split('/').any(|p| p == "..") {
        bail!("Archive entry contains an unsafe path segment: {name}");
    }
    Ok(normalized
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/"))
}

pub fn validate(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("Input file does not exist.");
    }
    if extension(path) != "epub" {
        bail!("Input file must use the .epub extension.");
    }
    let mut archive = ZipArchive::new(File::open(path)?)
        .map_err(|_| anyhow::anyhow!("Input file is not a valid ZIP archive."))?;
    let mut mimetype = String::new();
    archive
        .by_name("mimetype")
        .map_err(|_| anyhow::anyhow!("EPUB archive is missing the mimetype file."))?
        .read_to_string(&mut mimetype)?;
    if mimetype.trim() != "application/epub+zip" {
        bail!("EPUB mimetype is invalid.");
    }
    if archive.by_name("META-INF/container.xml").is_err() {
        bail!("EPUB archive is missing META-INF/container.xml.");
    }
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        if !entry.is_dir() {
            entry_name(entry.name())?;
        }
    }
    Ok(())
}

pub fn extract(path: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut archive = ZipArchive::new(File::open(path)?)?;
    let mut targets = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let relative = entry_name(entry.name())?;
        if relative.is_empty() {
            continue;
        }
        let key: String = relative.nfc().case_fold().collect();
        let target = targets
            .entry(key)
            .or_insert_with(|| destination.join(relative));
        fs::create_dir_all(target.parent().unwrap())?;
        std::io::copy(&mut entry, &mut File::create(target)?)?;
    }
    Ok(())
}

pub fn write(root: &Path, output: &Path) -> Result<()> {
    if !root.join("mimetype").is_file() {
        bail!("Cannot write EPUB without a mimetype file.");
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut archive = ZipWriter::new(File::create(output)?);
    let mut files = walkdir::WalkDir::new(root)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    files.retain(|p| p.file_type().is_file() && p.path() != root.join("mimetype"));
    files.sort_by(|a, b| a.path().cmp(b.path()));
    let options = SimpleFileOptions::default()
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    archive.start_file(
        "mimetype",
        options.compression_method(CompressionMethod::Stored),
    )?;
    archive.write_all(&fs::read(root.join("mimetype"))?)?;
    for file in files {
        let name = file
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        entry_name(&name)?;
        archive.start_file(
            name,
            options
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(6)),
        )?;
        std::io::copy(&mut File::open(file.path())?, &mut archive)?;
    }
    archive.finish()?;
    Ok(())
}

pub fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
pub fn dirname(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn join(dir: &str, href: &str) -> Result<String> {
    if href.starts_with('/') {
        bail!("Manifest href escapes the EPUB root: {href}");
    }
    let combined = format!("{dir}/{href}");
    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    bail!("Manifest href escapes the EPUB root: {href}");
                }
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

pub fn relative(from_file: &str, to_file: &str) -> String {
    let from: Vec<_> = dirname(from_file)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let to: Vec<_> = to_file.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut result = vec![".."; from.len() - common];
    result.extend_from_slice(&to[common..]);
    if result.is_empty() {
        ".".into()
    } else {
        result.join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_and_relative_links() {
        for name in ["../evil", "/etc/passwd", "C:\\book", "OEBPS/../../evil"] {
            assert!(entry_name(name).is_err(), "{name}");
        }
        assert_eq!(
            entry_name("OEBPS\\Text\\chapter.xhtml").unwrap(),
            "OEBPS/Text/chapter.xhtml"
        );
        assert_eq!(
            relative("OEBPS/Text/chapter.xhtml", "Styles/style.css"),
            "../../Styles/style.css"
        );
        assert!(join("OEBPS", "../../escape").is_err());
    }
}
