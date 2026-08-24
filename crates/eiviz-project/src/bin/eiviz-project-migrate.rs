use eiviz_project::{load, save_atomic};
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage: eiviz-project-migrate <input-project.json> --output <new-project.json>\n\
         The input is never modified. Review the output before replacing a project."
    );
    std::process::exit(2);
}

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let input = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| usage());
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--output")) {
        usage();
    }
    let output = arguments
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| usage());
    if arguments.next().is_some() || input == output {
        usage();
    }

    let result = load(&input).and_then(|project| save_atomic(&project, &output));
    if let Err(error) = result {
        eprintln!("eiviz-project-migrate: {error}");
        std::process::exit(1);
    }
    println!("{}", output.display());
}
