use alters_save_core::sav::SaveFile;
use alters_save_core::{items, resources, sav};
use proptest::prelude::*;

proptest! {
    #[test]
    fn parse_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = SaveFile::parse(&bytes);
    }

    #[test]
    fn resources_never_panic_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = resources::containers(&bytes);
    }

    #[test]
    fn inventory_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = items::inventory(&bytes, sav::ArchiveVersion::V3);
        let _ = items::inventory(&bytes, sav::ArchiveVersion::V2);
    }
}

fn fixture() -> Vec<u8> {
    let path = format!("{}/../test-data/act0-day1.sav", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(path).expect("fixture present")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn parse_never_panics_on_mutated_fixture(
        offsets in proptest::collection::vec(0usize..97_000, 1..32),
        values in proptest::collection::vec(any::<u8>(), 32),
    ) {
        let mut bytes = fixture();
        for (index, offset) in offsets.iter().enumerate() {
            if *offset < bytes.len() {
                bytes[*offset] = values[index % values.len()];
            }
        }
        if let Ok(save) = SaveFile::parse(&bytes) {
            let _ = resources::containers(&save.body);
            let _ = items::inventory(&save.body, save.archive_version());
            let _ = save.to_bytes();
        }
    }
}
