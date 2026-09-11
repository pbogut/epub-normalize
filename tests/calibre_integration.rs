mod support;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::*;

// calibredb uses a process-wide writer lock even for separate libraries.
static CALIBRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Library {
    temp: tempfile::TempDir,
    root: PathBuf,
    config: PathBuf,
}
impl Library {
    fn new() -> Option<Self> {
        if Command::new("calibredb").arg("--version").output().is_err() {
            eprintln!("Skipping real Calibre test: calibredb is unavailable.");
            return None;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        let config = temp.path().join("config");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&config).unwrap();
        Some(Self { temp, root, config })
    }
    fn db(&self, args: &[&str]) -> String {
        let output = Command::new("calibredb")
            .args(args)
            .arg("--with-library")
            .arg(&self.root)
            .env("CALIBRE_CONFIG_DIRECTORY", &self.config)
            .env("QT_QPA_PLATFORM", "offscreen")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        stdout(&output)
    }
    fn rows(&self) -> Vec<Value> {
        serde_json::from_str(&self.db(&["list", "--for-machine", "--fields", "title,tags,formats"]))
            .unwrap()
    }
    fn command(&self) -> Command {
        let mut c = cli(self.temp.path());
        c.arg("--with-library")
            .arg(&self.root)
            .env("CALIBRE_CONFIG_DIRECTORY", &self.config)
            .env("QT_QPA_PLATFORM", "offscreen");
        c
    }
    fn snapshot(&self) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
        self.rows()
            .into_iter()
            .map(|row| {
                (
                    row["title"].as_str().unwrap().to_owned(),
                    row["formats"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| {
                            let path = Path::new(v.as_str().unwrap());
                            (
                                path.extension().unwrap().to_string_lossy().into_owned(),
                                fs::read(path).unwrap(),
                            )
                        })
                        .collect(),
                )
            })
            .collect()
    }
}

#[test]
fn real_calibre_keeps_original_metadata_and_id_on_rerun() {
    let _guard = CALIBRE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(library) = Library::new() else {
        return;
    };
    let source = book(library.temp.path(), "input.epub");
    library.db(&[
        "add",
        source.to_str().unwrap(),
        "--title",
        "Imperium Czerni",
        "--tags",
        "keep-this-tag",
    ]);
    let before = library.rows().remove(0);
    let registered = PathBuf::from(before["formats"][0].as_str().unwrap());
    let original = fs::read(&registered).unwrap();
    let output = library
        .command()
        .args(["--calibre", "Imperium Czerni", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read(&registered).unwrap(), original);
    assert_eq!(library.rows()[0]["formats"].as_array().unwrap().len(), 1);
    for _ in 0..2 {
        let output = library
            .command()
            .args(["--calibre", "Imperium Czerni"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let rows = library.rows();
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row["id"], before["id"]);
        assert_eq!(row["title"], "Imperium Czerni");
        assert_eq!(row["tags"], serde_json::json!(["keep-this-tag"]));
        let snapshot = library.snapshot();
        let formats = &snapshot["Imperium Czerni"];
        assert_eq!(formats.len(), 2);
        assert_eq!(formats["original_epub"], original);
        assert_ne!(formats["epub"], original);
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&formats["epub"])).unwrap();
        assert!(archive.by_name("OEBPS/Styles/epub-optimizer.css").is_ok());
        assert!(archive.by_name("OEBPS/Styles/old.css").is_err());
    }
}
#[test]
fn real_calibre_batch_filters_exact_formats_and_skips_finished_books() {
    let _guard = CALIBRE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(library) = Library::new() else {
        return;
    };
    let source = book(library.temp.path(), "input.epub");
    for title in [
        "Eligible one",
        "Eligible two",
        "Already backed up",
        "Original only",
    ] {
        library.db(&["add", source.to_str().unwrap(), "--title", title]);
    }
    let text = library.temp.path().join("text.txt");
    fs::write(&text, "No EPUB").unwrap();
    library.db(&["add", text.to_str().unwrap(), "--title", "Text only"]);
    let backup = library.temp.path().join("backup.original_epub");
    fs::copy(&source, &backup).unwrap();
    for row in library.rows() {
        let title = row["title"].as_str().unwrap();
        let id = row["id"].to_string();
        if matches!(title, "Already backed up" | "Original only") {
            library.db(&["add_format", &id, backup.to_str().unwrap()]);
        }
        if title == "Original only" {
            library.db(&["remove_format", &id, "EPUB"]);
        }
    }
    let before = library.snapshot();
    let rows = library.rows();
    let first = rows.iter().find(|r| r["title"] == "Eligible one").unwrap();
    let extra = Path::new(first["formats"][0].as_str().unwrap())
        .with_file_name("Unregistered alternate.epub");
    fs::write(&extra, b"not an epub").unwrap();
    let output = library
        .command()
        .args(["--calibre", "--all", "--dry-run"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("2 previewed, 0 failed"));
    assert_eq!(library.snapshot(), before);
    let output = library
        .command()
        .args(["--calibre", "--all"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("2 processed, 0 failed"));
    let after = library.snapshot();
    for title in ["Eligible one", "Eligible two"] {
        assert_eq!(after[title].len(), 2);
        assert_eq!(after[title]["original_epub"], before[title]["epub"]);
        assert_ne!(after[title]["epub"], before[title]["epub"]);
    }
    for title in ["Already backed up", "Original only", "Text only"] {
        assert_eq!(after[title], before[title]);
    }
    assert_eq!(fs::read(extra).unwrap(), b"not an epub");
    let output = library
        .command()
        .args(["--calibre", "--all"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(stdout(&output).contains("No books"));
    assert_eq!(library.snapshot(), after);
}
