use alters_save_core::sav::{ArchiveVersion, SaveFile};
use alters_save_core::{items, resources, verify};

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/../test-data/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(path).expect("fixture present")
}

#[test]
fn act0_fixture_is_v3_and_verifies() {
    let bytes = fixture("act0-day1.sav");
    let save = SaveFile::parse(&bytes).expect("parses");
    assert_eq!(save.archive_version(), ArchiveVersion::V3);
    assert!(!resources::containers(&save.body).is_empty());
    let verify::Outcome::Pass(summary) = verify::verify(&bytes) else {
        panic!("expected pass");
    };
    assert!(summary.contains("injection"), "{summary}");
}

#[test]
fn act2_fixture_is_v2_and_verifies() {
    let bytes = fixture("act2-day54.sav");
    let save = SaveFile::parse(&bytes).expect("parses");
    assert_eq!(save.archive_version(), ArchiveVersion::V2);
    let inventory =
        items::inventory(&save.body, save.archive_version()).expect("v2 inventory lists");
    assert!(!inventory.stacks.is_empty());
    let verify::Outcome::Pass(summary) = verify::verify(&bytes) else {
        panic!("expected pass");
    };
    assert!(summary.contains("injection skipped"), "{summary}");
}

#[test]
fn v2_injection_is_refused() {
    let bytes = fixture("act2-day54.sav");
    let save = SaveFile::parse(&bytes).expect("parses");
    let result = items::add_stack(
        &save.body,
        save.archive_version(),
        &items::ItemClass("BridgePylon".to_owned()),
        4,
    );
    assert!(matches!(
        result,
        Err(alters_save_core::Error::UnsupportedArchiveVersion(2))
    ));
}

#[test]
fn corpus_env_dir_all_pass() {
    let Ok(dir) = std::env::var("ALTERS_CORPUS_DIR") else {
        eprintln!("ALTERS_CORPUS_DIR unset; skipping corpus test");
        return;
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).expect("corpus dir readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|ext| ext != "sav") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("readable");
        match verify::verify(&bytes) {
            verify::Outcome::Pass(_) | verify::Outcome::NotWorldSave => checked += 1,
            verify::Outcome::Fail(reason) => panic!("{}: {reason}", path.display()),
        }
    }
    assert!(checked > 0, "corpus dir contained no .sav files");
}
