use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _};

use alters_save_core::resources;
use alters_save_core::sav::SaveFile;

fn usage() -> ! {
    eprintln!(
        "usage:\n  alters-save-cli corpus <dir>       roundtrip-check every .sav in <dir>\n  alters-save-cli show <file.sav>    list resource containers"
    );
    std::process::exit(2)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "corpus" => corpus(Path::new(path)),
        [command, path] if command == "show" => show(Path::new(path)),
        _ => usage(),
    }
}

fn show(path: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let save = SaveFile::parse(&bytes)?;
    println!(
        "{}: prefix {} bytes, body {} bytes",
        path.display(),
        save.prefix().len(),
        save.body.len()
    );
    for container in resources::containers(&save.body) {
        println!(
            "  {:<20} amount {:>6}   second {:>6}",
            container.resource.0, container.amount, container.second
        );
    }
    Ok(())
}

enum Verdict {
    Pass(String),
    Skip,
    Fail(String),
}

struct Outcome {
    name: String,
    verdict: Verdict,
}

fn verdict_for(path: &Path) -> Verdict {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => return Verdict::Fail(format!("read: {error}")),
    };
    match alters_save_core::verify::verify(&bytes) {
        alters_save_core::verify::Outcome::Pass(summary) => Verdict::Pass(summary),
        alters_save_core::verify::Outcome::NotWorldSave => Verdict::Skip,
        alters_save_core::verify::Outcome::Fail(reason) => Verdict::Fail(reason),
    }
}

fn corpus(dir: &Path) -> anyhow::Result<()> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sav"))
        .collect();
    entries.sort();

    let outcomes: Vec<Outcome> = entries
        .iter()
        .map(|path| Outcome {
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            verdict: verdict_for(path),
        })
        .collect();

    let mut passed = 0_usize;
    let mut skipped = 0_usize;
    let mut failed = 0_usize;
    for Outcome { name, verdict } in &outcomes {
        match verdict {
            Verdict::Pass(summary) => {
                passed += 1;
                println!("PASS {name}: {summary}");
            }
            Verdict::Skip => {
                skipped += 1;
                println!("SKIP {name}: not a world save");
            }
            Verdict::Fail(reason) => {
                failed += 1;
                println!("FAIL {name}: {reason}");
            }
        }
    }
    println!(
        "\n{passed} passed, {skipped} skipped, {failed} failed, {} total",
        outcomes.len()
    );
    if failed > 0 {
        bail!("{failed} corpus files failed");
    }
    Ok(())
}
