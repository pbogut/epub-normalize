use crate::{
    archive,
    cli::{self, Args},
    core::{self, Options},
    process::{self, Interrupted},
};
use anyhow::{Result, anyhow, bail};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};
use unicode_casefold::UnicodeCaseFold;

pub struct Book {
    pub id: i64,
    pub title: String,
    pub authors: String,
    pub formats: BTreeMap<String, PathBuf>,
    pub directory: PathBuf,
}
pub fn run_calibredb(arguments: &[String], library: Option<&Path>) -> Result<String> {
    let mut command = Command::new("calibredb");
    command.args(arguments);
    if let Some(library) = library {
        command.arg("--with-library").arg(library);
    }
    let result = process::capture(&mut command, None, false, None).map_err(|e| {
        if e.downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            anyhow!("calibredb was not found. Install Calibre and put it on PATH.")
        } else {
            e
        }
    })?;
    if !result.status.success() {
        let fallback = format!("exit {}", result.status.code().unwrap_or(-1));
        let detail = if !result.stderr.trim().is_empty() {
            result.stderr.trim()
        } else if !result.stdout.trim().is_empty() {
            result.stdout.trim()
        } else {
            &fallback
        };
        bail!("calibredb {} failed: {detail}", arguments[0]);
    }
    Ok(result.stdout)
}
pub fn pick(labels: &[String], prompt: &str) -> Result<usize> {
    if labels.len() == 1 {
        return Ok(0);
    }
    let rows: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            format!(
                "{i}\t{}",
                l.split_whitespace().collect::<Vec<_>>().join(" ")
            )
        })
        .collect();
    let mut command = Command::new("fzf");
    command
        .args([
            "--no-multi",
            "--delimiter=\t",
            "--with-nth=2..",
            &format!("--prompt={prompt}"),
        ])
        .env_remove("FZF_DEFAULT_OPTS")
        .env_remove("FZF_DEFAULT_OPTS_FILE");
    let output = process::capture(
        &mut command,
        Some(&format!("{}\n", rows.join("\n"))),
        true,
        None,
    )
    .map_err(|e| {
        if e.downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            anyhow!("Multiple matches found. Install fzf to choose one.")
        } else {
            e
        }
    })?;
    if matches!(output.status.code(), Some(1 | 130)) {
        return Err(Interrupted.into());
    }
    if !output.status.success() {
        bail!(
            "fzf failed with exit code {}.",
            output.status.code().unwrap_or(-1)
        );
    }
    rows.iter()
        .position(|r| r == output.stdout.trim_end_matches('\n'))
        .ok_or_else(|| anyhow!("fzf returned an invalid selection."))
}
fn entry_id(entry: &Value) -> Option<i64> {
    entry.get("id").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}
fn title(entry: &Value) -> String {
    entry
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .into()
}
fn authors(entry: &Value) -> String {
    entry
        .get("authors")
        .and_then(Value::as_str)
        .unwrap_or("")
        .into()
}
fn list_entries(query: &str, library: Option<&str>) -> Result<(Vec<Value>, Option<PathBuf>)> {
    let root = if let Some(library) = library.filter(|s| !s.is_empty()) {
        let expanded = if library == "~" || library.starts_with("~/") {
            std::env::var("HOME")
                .map(|home| format!("{home}{}", &library[1..]))
                .unwrap_or_else(|_| library.into())
        } else {
            library.into()
        };
        let root = cli::resolve(Path::new(&expanded))?;
        if !root.join("metadata.db").is_file() {
            bail!(
                "Not a local Calibre library, metadata.db is missing: {}",
                root.display()
            );
        }
        Some(root)
    } else {
        None
    };
    let raw = run_calibredb(
        &[
            "list",
            "--for-machine",
            "--fields",
            "title,authors,formats,cover",
            "--search",
            query,
        ]
        .map(str::to_string),
        root.as_deref(),
    )?;
    let entries: Vec<Value> = serde_json::from_str(&raw)
        .map_err(|_| anyhow!("calibredb returned an invalid book list."))?;
    if entries
        .iter()
        .any(|e| entry_id(e).is_none() || e.get("title").is_none())
    {
        bail!("calibredb returned an invalid book list.");
    }
    Ok((entries, root))
}
fn entry_book(entry: &Value, root: Option<&Path>) -> Result<(Book, PathBuf)> {
    let paths: Vec<_> = entry
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(PathBuf::from)
        .collect();
    let formats = paths
        .iter()
        .map(|p| (archive::extension(p).to_uppercase(), p.clone()))
        .collect();
    let cover = entry
        .get("cover")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let first = paths.first().or(cover.as_ref()).ok_or_else(|| {
        anyhow!(
            "No local files found for book {}: {}",
            entry_id(entry).unwrap(),
            title(entry)
        )
    })?;
    let directory = first.parent().unwrap_or(Path::new(".")).to_path_buf();
    let root = root
        .map(Path::to_path_buf)
        .or_else(|| {
            directory
                .ancestors()
                .skip(1)
                .find(|p| p.join("metadata.db").is_file())
                .map(Path::to_path_buf)
        })
        .ok_or_else(|| {
            anyhow!(
                "Could not locate the Calibre library containing {}",
                directory.display()
            )
        })?;
    Ok((
        Book {
            id: entry_id(entry).unwrap(),
            title: title(entry),
            authors: authors(entry),
            formats,
            directory,
        },
        root,
    ))
}
pub fn normalize(query: &str, args: &Args) -> Result<i32> {
    if args.all {
        return normalize_all(args);
    }
    let (entries, root) = list_entries(query, args.with_library.as_deref())?;
    if entries.is_empty() {
        bail!("No Calibre books match: {query}");
    }
    let labels: Vec<_> = entries
        .iter()
        .map(|e| format!("{} | {} | {}", entry_id(e).unwrap(), title(e), authors(e)))
        .collect();
    let (book, root) = entry_book(&entries[pick(&labels, "Book> ")?], root.as_deref())?;
    let mut candidates: Vec<_> = fs::read_dir(&book.directory)?
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && archive::extension(p) == "epub"
                && !p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase()
                    .ends_with(".original.epub")
        })
        .collect();
    candidates.sort_by_cached_key(|p| {
        p.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .case_fold()
            .collect::<String>()
    });
    if candidates.is_empty() {
        bail!("No EPUB files found for book {}: {}", book.id, book.title);
    }
    let labels = candidates
        .iter()
        .map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    let source = &candidates[pick(&labels, "EPUB> ")?];
    println!("Selected {}: {} | {}", book.id, book.title, book.authors);
    normalize_book(&book, source, &root, args)?;
    Ok(0)
}
fn normalize_all(args: &Args) -> Result<i32> {
    let (entries, mut root) = list_entries(
        "formats:\"=EPUB\" and not formats:\"=ORIGINAL_EPUB\"",
        args.with_library.as_deref(),
    )?;
    if entries.is_empty() {
        println!("No books with EPUB and without ORIGINAL_EPUB need processing.");
        return Ok(0);
    }
    let mut failed = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        process::check_interrupt()?;
        println!(
            "[{}/{}] {} | {}",
            index + 1,
            entries.len(),
            title(entry),
            authors(entry)
        );
        std::io::stdout().flush()?;
        let result = (|| {
            let (book, library) = entry_book(entry, root.as_deref())?;
            root = Some(library.clone());
            let source = book
                .formats
                .get("EPUB")
                .ok_or_else(|| anyhow!("Registered EPUB file is missing."))?;
            normalize_book(&book, source, &library, args)
        })();
        if let Err(error) = result {
            if error.is::<Interrupted>() {
                return Err(error);
            }
            let id = entry_id(entry).unwrap();
            failed.push(id);
            eprintln!("Book {id} ({}): error: {error}", title(entry));
        }
    }
    println!(
        "Batch complete: {} {}, {} failed.",
        entries.len() - failed.len(),
        if args.dry_run {
            "previewed"
        } else {
            "processed"
        },
        failed.len()
    );
    if !failed.is_empty() {
        eprintln!(
            "Failed book IDs: {}",
            failed
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    Ok(i32::from(!failed.is_empty()))
}
fn normalize_book(book: &Book, source: &Path, root: &Path, args: &Args) -> Result<()> {
    println!("EPUB: {}", source.display());
    let current = book
        .formats
        .get("EPUB")
        .map(PathBuf::as_path)
        .unwrap_or(source);
    let original = book.formats.get("ORIGINAL_EPUB");
    if args.dry_run {
        let preview = core::preview(source, args.preserve_publisher_css)?;
        println!(
            "[dry run] {} document(s), {} stylesheet/font entry(s) to remove, {} image(s) preserved",
            preview.content_documents, preview.stylesheets_and_fonts, preview.images_preserved
        );
        if let Some(original) = original {
            println!("Would keep ORIGINAL_EPUB: {}", original.display());
        } else {
            println!("Would back up as ORIGINAL_EPUB: {}", current.display());
        }
        println!(
            "Would replace EPUB on Calibre book {} using add_format.",
            book.id
        );
        for warning in preview.warnings.iter().chain(&preview.image_diagnostics) {
            eprintln!("warning: {warning}");
        }
        return Ok(());
    }
    let temp = tempfile::Builder::new()
        .prefix("epub-normalize-calibre-")
        .tempdir()?;
    let backup = temp
        .path()
        .join(current.with_extension("original_epub").file_name().unwrap());
    if original.is_none() {
        fs::copy(current, &backup)?;
    }
    let progress = |message: &str| eprintln!("{message}");
    let filename = current.file_name().unwrap_or_default().to_string_lossy();
    let result = core::optimize(
        source,
        temp.path(),
        &Options {
            output_filename: Some(&filename),
            preserve_publisher_css: args.preserve_publisher_css,
            progress: args.verbose.then_some(&progress),
            ..Default::default()
        },
    )?;
    process::check_interrupt()?;
    if let Some(original) = original {
        println!("Kept existing ORIGINAL_EPUB: {}", original.display());
    } else {
        run_calibredb(
            &[
                "add_format".into(),
                book.id.to_string(),
                backup.to_string_lossy().into_owned(),
                "--dont-replace".into(),
            ],
            Some(root),
        )?;
        println!("Saved ORIGINAL_EPUB backup in Calibre.");
    }
    run_calibredb(
        &[
            "add_format".into(),
            book.id.to_string(),
            result.output_path.to_string_lossy().into_owned(),
        ],
        Some(root),
    )
    .map_err(|e| {
        if e.is::<Interrupted>() {
            e
        } else {
            anyhow!("{e}\nThe ORIGINAL_EPUB backup is retained in Calibre.")
        }
    })?;
    println!(
        "Updated Calibre book {}. Formats: EPUB, ORIGINAL_EPUB.",
        book.id
    );
    println!("  EPUBCheck: {}", result.validation_outcome);
    for warning in result.warnings.iter().chain(&result.image_diagnostics) {
        eprintln!("warning: {warning}");
    }
    Ok(())
}
