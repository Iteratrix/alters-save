fn main() {
    let path = std::env::args().nth(1).expect("path");
    let bytes = std::fs::read(path).expect("read");
    let save = alters_save_core::sav::SaveFile::parse(&bytes).expect("parse");
    let version = save.archive_version();
    println!("version {version:?}");
    match alters_save_core::time::game_time(&save.body, version) {
        Ok(t) => println!("time: day {} hour {} minute {}", t.day, t.hour, t.minute),
        Err(e) => println!("time error: {e}"),
    }
    match alters_save_core::research::research(&save.body, version) {
        Ok(r) => println!(
            "research: {} unlocked, {} discovered",
            r.unlocked.len(),
            r.discovered.len()
        ),
        Err(e) => println!("research error: {e}"),
    }
    match alters_save_core::alters::alters(&save.body) {
        Ok(a) => {
            for alter in a {
                println!(
                    "alter {}: {} emotions, {} radiation",
                    alter.name,
                    alter.emotions.len(),
                    alter.radiation.len()
                );
            }
        }
        Err(e) => println!("alters error: {e}"),
    }
    let deadlines = alters_save_core::quests::deadlines(&save.body, version);
    println!("{} quest deadlines", deadlines.len());
}
