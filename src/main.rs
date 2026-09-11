use clap::Parser;
use epub_normalize::{cli, process};
fn main() {
    if let Err(error) = ctrlc::set_handler(|| {
        process::INTERRUPTED.store(true, std::sync::atomic::Ordering::Relaxed)
    }) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
    let code = cli::run(cli::Args::parse());
    std::process::exit(code);
}
