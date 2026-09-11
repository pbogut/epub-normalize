mod support;
use std::fs;
use support::*;

#[test]
fn command_runs_outside_checkout_and_preserves_input() {
    let temp = tempfile::tempdir().unwrap();
    let path = book(temp.path(), "A book.epub");
    let original = fs::read(&path).unwrap();
    let output = cli(temp.path()).arg("A book.epub").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "A book.epub -> A book-normalized.epub\n  1 document(s), 3 stylesheet/font entry(s) removed, 0 image(s) preserved\n  EPUBCheck: unavailable\n"
    );
    assert_eq!(fs::read(path).unwrap(), original);
    assert!(temp.path().join("A book-normalized.epub").is_file());
}
#[test]
fn dry_run_has_no_output_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let output = cli(temp.path())
        .args([
            "book.epub",
            "-n",
            "-d",
            "new/subdir",
            "--preserve-publisher-css",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!temp.path().join("new").exists());
    assert!(stdout(&output).contains("[dry run]"));
    assert!(stdout(&output).contains("1 stylesheet/font entry(s) to remove"));
}
#[test]
fn force_is_required_and_failed_force_keeps_output() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let destination = temp.path().join("book-normalized.epub");
    fs::write(&destination, b"existing").unwrap();
    let output = cli(temp.path()).arg("book.epub").output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("Use --force"));
    fs::write(temp.path().join("book.epub"), b"bad").unwrap();
    let output = cli(temp.path()).args(["book.epub", "-f"]).output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(destination).unwrap(), b"existing");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
}
#[test]
fn batch_continues_after_a_bad_input() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "good.epub");
    fs::write(temp.path().join("bad.epub"), "bad").unwrap();
    let output = cli(temp.path())
        .args(["bad.epub", "good.epub", "-d", "out"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(temp.path().join("out/good-normalized.epub").exists());
    assert!(!temp.path().join("out/bad-normalized.epub").exists());
    assert!(stderr(&output).contains("bad.epub: error:"));
}
#[test]
fn output_conflicts_are_rejected_before_processing() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    book(temp.path(), "book-normalized.epub");
    let output = cli(temp.path())
        .args(["book.epub", "book-normalized.epub", "-f"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    for name in ["one", "two"] {
        fs::create_dir(temp.path().join(name)).unwrap();
        book(&temp.path().join(name), "book.epub");
    }
    let output = cli(temp.path())
        .args(["one/book.epub", "two/book.epub", "-d", "out"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(!temp.path().join("out").exists());
    let output = cli(temp.path())
        .args(["book.epub", "-o", "book.epub", "-f"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}
#[cfg(unix)]
#[test]
fn input_symlinks_hardlinks_and_parent_aliases_are_protected() {
    let temp = tempfile::tempdir().unwrap();
    let source = book(temp.path(), "book.epub");
    let original = fs::read(&source).unwrap();
    std::os::unix::fs::symlink("book.epub", temp.path().join("symbolic.epub")).unwrap();
    fs::hard_link(&source, temp.path().join("hard.epub")).unwrap();
    fs::create_dir(temp.path().join("directory")).unwrap();
    std::os::unix::fs::symlink(temp.path(), temp.path().join("parent")).unwrap();
    for name in [
        "symbolic.epub",
        "hard.epub",
        "directory/../book.epub",
        "parent/book.epub",
    ] {
        let output = cli(temp.path())
            .args(["book.epub", "-f", "-o", name])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{name}: {}", stderr(&output));
    }
    assert_eq!(fs::read(source).unwrap(), original);
}
#[test]
fn invalid_argument_combinations_return_two() {
    let temp = tempfile::tempdir().unwrap();
    for args in [
        vec![],
        vec!["--all"],
        vec!["--calibre"],
        vec!["--calibre", " "],
        vec!["--calibre", "query", "--all"],
        vec!["--calibre", "query", "-f"],
        vec!["--calibre", "query", "book.epub"],
        vec!["--calibre", "query", "-o", "out.epub"],
        vec!["--with-library", "/missing", "book.epub"],
        vec!["book.epub", "-o", "out.zip"],
        vec!["one.epub", "two.epub", "-o", "out.epub"],
        vec!["book.epub", "-o", "out.epub", "-d", "out"],
    ] {
        let output = cli(temp.path()).args(&args).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}
#[test]
fn version_verbose_and_preserve_css_flags_work() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let version = cli(temp.path()).arg("--version").output().unwrap();
    assert_eq!(stdout(&version), "epub-normalize 0.1.0\n");
    let output = cli(temp.path())
        .args([
            "book.epub",
            "-v",
            "-o",
            "chosen.EPUB",
            "--preserve-publisher-css",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("book.epub: Validated EPUB archive."));
    let mut archive =
        zip::ZipArchive::new(fs::File::open(temp.path().join("chosen.EPUB")).unwrap()).unwrap();
    assert!(archive.by_name("OEBPS/Styles/old.css").is_ok());
    assert!(archive.by_name("OEBPS/Fonts/publisher.woff2").is_ok());
}
#[cfg(unix)]
#[test]
fn concurrent_output_is_not_overwritten() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let checker = script(
        temp.path(),
        "epubcheck",
        "printf racer > \"$DESTINATION\"\nprintf '%s\\n' '{\"messages\":[]}'",
    );
    let output = cli(temp.path())
        .arg("book.epub")
        .env("EPUBCHECK_EXECUTABLE", checker)
        .env("DESTINATION", temp.path().join("book-normalized.epub"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        fs::read(temp.path().join("book-normalized.epub")).unwrap(),
        b"racer"
    );
}
#[cfg(unix)]
#[test]
fn epubcheck_timeout_remains_advisory_without_a_baseline() {
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let checker = script(temp.path(), "epubcheck", "exec sleep 20");
    let output = cli(temp.path())
        .arg("book.epub")
        .env("EPUBCHECK_EXECUTABLE", checker)
        .env("EPUBCHECK_TIMEOUT", "0.02")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("EPUBCheck: unavailable"));
}
#[cfg(unix)]
#[test]
fn interrupt_returns_130_and_cleans_staging() {
    use std::{
        process::{Command, Stdio},
        thread,
        time::Duration,
    };
    let temp = tempfile::tempdir().unwrap();
    book(temp.path(), "book.epub");
    let checker = script(
        temp.path(),
        "epubcheck",
        "printf ready > \"$READY\"\nexec sleep 20",
    );
    let ready = temp.path().join("ready");
    let child = cli(temp.path())
        .arg("book.epub")
        .env("EPUBCHECK_EXECUTABLE", checker)
        .env("READY", &ready)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..500 {
        if ready.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists());
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(130));
    assert!(stderr(&output).contains("Interrupted."));
    assert!(!temp.path().join("book-normalized.epub").exists());
    assert!(!fs::read_dir(temp.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".epub-normalize-")
    }));
}
