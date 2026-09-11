use crate::{
    archive,
    core::{self, Options},
    process::{self, Interrupted},
};
use anyhow::{Result, bail};
use clap::{CommandFactory, Parser, error::ErrorKind};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Parser, Debug)]
#[command(
    name = "epub-normalize",
    about = "Normalize EPUB typography locally.",
    version,
    disable_version_flag = true,
    infer_long_args = true,
    args_override_self = true
)]
pub struct Args {
    #[arg(value_name = "inputs", help = "EPUB files to normalize")]
    pub inputs: Vec<PathBuf>,
    #[arg(long, value_name = "QUERY", num_args = 0..=1, default_missing_value = "", help = "search a Calibre library and pick a book, or use with --all")]
    pub calibre: Option<String>,
    #[arg(long, help = "with --calibre, process EPUBs without ORIGINAL_EPUB")]
    pub all: bool,
    #[arg(
        long,
        value_name = "PATH",
        help = "local Calibre library, with --calibre"
    )]
    pub with_library: Option<String>,
    #[arg(
        short,
        long,
        conflicts_with = "output_dir",
        help = "output filename, for a single input"
    )]
    pub output: Option<PathBuf>,
    #[arg(short = 'd', long, help = "directory for normalized books")]
    pub output_dir: Option<PathBuf>,
    #[arg(short = 'n', long, help = "preview changes without writing")]
    pub dry_run: bool,
    #[arg(short = 'f', long, help = "replace existing output files")]
    pub force: bool,
    #[arg(short = 'v', long, help = "show processing steps")]
    pub verbose: bool,
    #[arg(
        long,
        help = "keep publisher styles and fonts alongside the normalization stylesheet"
    )]
    pub preserve_publisher_css: bool,
    #[arg(long, action = clap::ArgAction::Version)]
    pub version: (),
}
pub fn usage_error(message: impl Into<String>) -> clap::Error {
    Args::command().error(ErrorKind::ArgumentConflict, message.into())
}
impl Args {
    pub fn validate(&self) -> std::result::Result<(), clap::Error> {
        let error = if self.all && self.calibre.is_none() {
            Some("--all requires --calibre")
        } else if let Some(query) = &self.calibre {
            if self.all && !query.is_empty() {
                Some("use --calibre --all without a search query")
            } else if !self.all && query.trim().is_empty() {
                Some("--calibre requires a non-empty search query or --all")
            } else if !self.inputs.is_empty()
                || self.output.is_some()
                || self.output_dir.is_some()
                || self.force
            {
                Some(
                    "--calibre cannot be combined with input files, --output, --output-dir or --force",
                )
            } else {
                None
            }
        } else if self.inputs.is_empty() {
            Some("provide EPUB files or --calibre QUERY")
        } else if self.with_library.is_some() {
            Some("--with-library requires --calibre")
        } else {
            None
        };
        if let Some(error) = error {
            return Err(usage_error(error));
        }
        if let Some(output) = &self.output {
            if self.inputs.len() != 1 {
                return Err(usage_error(
                    "--output requires exactly one input; use --output-dir for multiple books",
                ));
            }
            if archive::extension(output) != "epub" {
                return Err(usage_error("--output must have an .epub extension"));
            }
        }
        Ok(())
    }
}

pub fn resolve(path: &Path) -> Result<PathBuf> {
    fn inner(path: &Path, depth: usize) -> Result<PathBuf> {
        if depth > 40 {
            bail!("Symlink loop from '{}'", path.display());
        }
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let mut resolved = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    resolved.pop();
                }
                _ => {
                    resolved.push(component.as_os_str());
                    match fs::read_link(&resolved) {
                        Ok(target) => {
                            let target = if target.is_absolute() {
                                target
                            } else {
                                resolved.parent().unwrap_or(Path::new("/")).join(target)
                            };
                            resolved = inner(&target, depth + 1)?;
                        }
                        Err(e)
                            if e.kind() == std::io::ErrorKind::NotFound
                                || e.kind() == std::io::ErrorKind::InvalidInput
                                || e.kind() == std::io::ErrorKind::NotADirectory => {}
                        Err(e) => return Err(e.into()),
                    }
                }
            }
        }
        Ok(resolved)
    }
    inner(path, 0)
}
pub fn jobs(args: &Args) -> std::result::Result<Vec<(PathBuf, PathBuf)>, clap::Error> {
    let inputs: BTreeSet<_> = args
        .inputs
        .iter()
        .map(|p| resolve(p))
        .collect::<Result<_>>()
        .map_err(|e| usage_error(e.to_string()))?;
    let mut destinations = BTreeSet::new();
    let mut jobs = Vec::new();
    for source in &args.inputs {
        let source: PathBuf = source.components().collect();
        let destination = args.output.clone().unwrap_or_else(|| {
            args.output_dir
                .as_deref()
                .unwrap_or_else(|| source.parent().unwrap_or(Path::new(".")))
                .join(format!(
                    "{}-normalized.epub",
                    source.file_stem().unwrap_or_default().to_string_lossy()
                ))
        });
        let resolved = resolve(&destination).map_err(|e| usage_error(e.to_string()))?;
        if inputs.contains(&resolved)
            || (destination.exists()
                && inputs.iter().any(|i| {
                    i.exists() && same_file::is_same_file(i, &destination).unwrap_or(false)
                }))
        {
            return Err(usage_error(format!(
                "output would overwrite an input file: {}",
                destination.display()
            )));
        }
        if !destinations.insert(resolved) {
            return Err(usage_error(format!(
                "multiple inputs would write to the same output: {}",
                destination.display()
            )));
        }
        jobs.push((source, destination));
    }
    Ok(jobs)
}
pub fn process_file(source: &Path, destination: &Path, args: &Args) -> Result<()> {
    process::check_interrupt()?;
    if args.dry_run {
        let preview = core::preview(source, args.preserve_publisher_css)?;
        println!(
            "{} -> {} [dry run]",
            source.display(),
            destination.display()
        );
        println!(
            "  {} document(s), {} stylesheet/font entry(s) to remove, {} image(s) preserved",
            preview.content_documents, preview.stylesheets_and_fonts, preview.images_preserved
        );
        for warning in preview.warnings.iter().chain(&preview.image_diagnostics) {
            eprintln!("{}: warning: {warning}", source.display());
        }
        return Ok(());
    }
    if !args.force && (destination.exists() || destination.is_symlink()) {
        bail!(
            "Output already exists: {}. Use --force to replace it.",
            destination.display()
        );
    }
    if !source.is_file() {
        bail!("Input file does not exist or is not a regular file.");
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let progress = |message: &str| eprintln!("{}: {message}", source.display());
    let temp = tempfile::Builder::new()
        .prefix(".epub-normalize-")
        .tempdir_in(parent)?;
    let filename = destination
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
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
    if args.force {
        fs::rename(&result.output_path, destination)?;
    } else {
        fs::hard_link(&result.output_path, destination)?;
    }
    println!("{} -> {}", source.display(), destination.display());
    println!(
        "  {} document(s), {} stylesheet/font entry(s) removed, {} image(s) preserved",
        result.content_documents_processed, result.stylesheets_replaced, result.images_preserved
    );
    println!("  EPUBCheck: {}", result.validation_outcome);
    for warning in result.warnings.iter().chain(&result.image_diagnostics) {
        eprintln!("{}: warning: {warning}", source.display());
    }
    Ok(())
}
pub fn run(args: Args) -> i32 {
    if let Err(error) = args.validate() {
        let _ = error.print();
        return 2;
    }
    if let Some(query) = &args.calibre {
        return match crate::calibre::normalize(query, &args) {
            Ok(code) => code,
            Err(error) => report_error(error, None),
        };
    }
    let jobs = match jobs(&args) {
        Ok(jobs) => jobs,
        Err(error) => {
            let _ = error.print();
            return 2;
        }
    };
    let mut failed = false;
    for (source, destination) in jobs {
        if let Err(error) = process_file(&source, &destination, &args) {
            let code = report_error(error, Some(&source));
            if code == 130 {
                return code;
            }
            failed = true;
        }
    }
    i32::from(failed)
}
fn report_error(error: anyhow::Error, source: Option<&Path>) -> i32 {
    if error.is::<Interrupted>() {
        eprintln!("Interrupted.");
        return 130;
    }
    if let Some(source) = source {
        eprintln!("{}: error: {error}", source.display());
    } else {
        eprintln!("error: {error}");
    }
    1
}
