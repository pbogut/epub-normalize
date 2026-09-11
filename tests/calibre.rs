#![cfg(unix)]
mod support;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::*;

struct Fake {
    temp: tempfile::TempDir,
    library: PathBuf,
    directory: PathBuf,
    source: PathBuf,
}
impl Fake {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let library = temp.path().join("library");
        let directory = library.join("Writer/Book (42)");
        fs::create_dir_all(&directory).unwrap();
        fs::write(library.join("metadata.db"), "").unwrap();
        let source = book(&directory, "book.epub");
        script(
            temp.path(),
            "calibredb",
            r#"
case "$1" in
list) printf '%s\n' "$@" > "$QUERY_LOG"; cat "$BOOK_LIST" ;;
add_format)
  printf '%s\n' "$@" >> "$IMPORT_LOG"
  case "$3" in
    *.original_epub) [ "$FAIL_FORMAT" = backup ] && { printf 'backup failed\n' >&2; exit 1; }; cp "$3" "$BACKUP_CAPTURE" ;;
    *) [ "$FAIL_FORMAT" = replacement ] && { printf 'replacement failed\n' >&2; exit 1; }; cp "$3" "$EPUB_CAPTURE" ;;
  esac ;;
*) exit 9 ;;
esac
"#,
        );
        let fake = Self {
            temp,
            library,
            directory,
            source,
        };
        fake.rows(json!([fake.row(42, "Book", &fake.source)]));
        fake
    }
    fn row(&self, id: i64, title: &str, source: &Path) -> Value {
        json!({"id": id, "title": title, "authors": "Writer", "formats": [source]})
    }
    fn rows(&self, rows: Value) {
        fs::write(self.temp.path().join("books.json"), rows.to_string()).unwrap();
    }
    fn command(&self) -> Command {
        let root = self.temp.path();
        let mut c = cli(root);
        c.env("PATH", path_with(root))
            .env("BOOK_LIST", root.join("books.json"))
            .env("QUERY_LOG", root.join("query.log"))
            .env("IMPORT_LOG", root.join("imports.log"))
            .env("BACKUP_CAPTURE", root.join("backup.bytes"))
            .env("EPUB_CAPTURE", root.join("epub.bytes"));
        c
    }
}
#[test]
fn single_match_pins_library_and_registers_backup_first() {
    let fake = Fake::new();
    let original = fs::read(&fake.source).unwrap();
    let output = fake
        .command()
        .args(["--calibre", "Book; $(touch injected)"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read(fake.temp.path().join("backup.bytes")).unwrap(),
        original
    );
    let log = fs::read_to_string(fake.temp.path().join("imports.log")).unwrap();
    assert!(log.find(".original_epub").unwrap() < log.rfind("book.epub").unwrap());
    assert!(log.contains("--dont-replace"));
    assert!(log.contains(&fake.library.to_string_lossy().to_string()));
    assert!(!fake.temp.path().join("injected").exists());
    assert!(
        fs::read_to_string(fake.temp.path().join("query.log"))
            .unwrap()
            .contains("Book; $(touch injected)")
    );
}
#[test]
fn dry_run_and_invalid_books_never_import() {
    let fake = Fake::new();
    let output = fake
        .command()
        .args(["--calibre", "Book", "-n"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Would back up as ORIGINAL_EPUB:"));
    assert!(!fake.temp.path().join("imports.log").exists());
    fs::write(&fake.source, b"invalid").unwrap();
    for extra in [vec![], vec!["-n"]] {
        let output = fake
            .command()
            .args(["--calibre", "Book"])
            .args(extra)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(!fake.temp.path().join("imports.log").exists());
    }
}
#[test]
fn import_failures_stop_in_order_and_report_retained_backup() {
    for failure in ["backup", "replacement"] {
        let fake = Fake::new();
        let output = fake
            .command()
            .args(["--calibre", "Book"])
            .env("FAIL_FORMAT", failure)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains(&format!("{failure} failed")));
        assert!(!fake.temp.path().join("epub.bytes").exists());
        let log = fs::read_to_string(fake.temp.path().join("imports.log")).unwrap();
        assert_eq!(
            log.lines().filter(|line| *line == "add_format").count(),
            if failure == "backup" { 1 } else { 2 }
        );
        if failure == "replacement" {
            assert!(stderr(&output).contains("backup is retained in Calibre"));
            assert!(fake.temp.path().join("backup.bytes").exists());
        }
    }
}
#[test]
fn existing_backup_is_retained() {
    let fake = Fake::new();
    let original = fake.directory.join("book.original_epub");
    fs::write(&original, b"first original").unwrap();
    fake.rows(json!([{"id": 42, "title": "Book", "authors": "Writer", "formats": [fake.source, original]}]));
    let output = fake.command().args(["--calibre", "Book"]).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Kept existing ORIGINAL_EPUB:"));
    assert!(!fake.temp.path().join("backup.bytes").exists());
    assert_eq!(fs::read(original).unwrap(), b"first original");
}
#[test]
fn alternate_selection_backs_up_registered_epub_and_ignores_fzf_defaults() {
    let fake = Fake::new();
    let original = fs::read(&fake.source).unwrap();
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/appendix.epub"),
        fake.directory.join("alternate.epub"),
    )
    .unwrap();
    fs::write(
        fake.directory.join("backup.original.epub"),
        b"must not select",
    )
    .unwrap();
    script(
        fake.temp.path(),
        "fzf",
        r#"[ -z "${FZF_DEFAULT_OPTS+x}" ] || exit 8
[ -z "${FZF_DEFAULT_OPTS_FILE+x}" ] || exit 8
IFS= read -r row
printf '%s\n' "$row""#,
    );
    let output = fake
        .command()
        .args(["--calibre", "Book"])
        .env("FZF_DEFAULT_OPTS", "--multi --filter=x")
        .env("FZF_DEFAULT_OPTS_FILE", "/bad")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("alternate.epub"));
    assert_eq!(
        fs::read(fake.temp.path().join("backup.bytes")).unwrap(),
        original
    );
}
#[test]
fn picker_cancellation_is_130_and_bad_output_is_rejected() {
    for script_body in ["exit 1", "exit 130", "printf 'invalid selection\\n'"] {
        let fake = Fake::new();
        fake.rows(json!([
            fake.row(42, "First", &fake.source),
            fake.row(43, "Second", &fake.source)
        ]));
        script(fake.temp.path(), "fzf", script_body);
        let output = fake.command().args(["--calibre", "Book"]).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(if script_body.starts_with("exit") {
                130
            } else {
                1
            })
        );
        assert!(!fake.temp.path().join("imports.log").exists());
    }
}
#[test]
fn batch_uses_registered_epubs_and_continues_after_failure() {
    let fake = Fake::new();
    let invalid = fake.directory.join("bad.epub");
    fs::write(&invalid, b"bad").unwrap();
    fake.rows(json!([
        fake.row(41, "Broken", &invalid),
        fake.row(42, "Book", &fake.source)
    ]));
    script(fake.temp.path(), "fzf", "exit 99");
    let output = fake
        .command()
        .args(["--calibre", "--all"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).contains("1 processed, 1 failed"));
    assert!(stderr(&output).contains("Failed book IDs: 41"));
    assert!(
        fs::read_to_string(fake.temp.path().join("query.log"))
            .unwrap()
            .contains("formats:\"=EPUB\" and not formats:\"=ORIGINAL_EPUB\"")
    );
    assert!(fake.temp.path().join("epub.bytes").exists());
}
#[test]
fn empty_batch_and_missing_library() {
    let fake = Fake::new();
    fake.rows(json!([]));
    let output = fake
        .command()
        .args(["--calibre", "--all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(stdout(&output).contains("No books"));
    let output = fake
        .command()
        .args(["--calibre", "query"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("No Calibre books match"));
    let output = fake
        .command()
        .args(["--calibre", "query", "--with-library", "missing"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(!fake.temp.path().join("missing").exists());
}
