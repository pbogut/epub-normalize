#![allow(dead_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.epub")
}
pub fn cli(root: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_epub-normalize"));
    c.current_dir(root)
        .env_remove("EPUBCHECK_EXECUTABLE")
        .env_remove("EPUBCHECK_JAR");
    c
}
pub fn book(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    fs::copy(fixture(), &path).unwrap();
    path
}
#[cfg(unix)]
pub fn script(root: &Path, name: &str, content: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{content}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}
pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}
pub fn path_with(root: &Path) -> std::ffi::OsString {
    std::env::join_paths(
        std::iter::once(root.to_path_buf()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .unwrap()
}
