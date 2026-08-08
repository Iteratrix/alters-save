# alters-save

Save editor for [The Alters](https://store.steampowered.com/app/1601570/The_Alters/)
(11 bit studios). Edit base resources and inventory items directly in your
browser - nothing is uploaded anywhere; all parsing runs locally as
WebAssembly.

**No documentation for this save format existed anywhere public.** The
format here was reverse-engineered from scratch against a corpus of 100+
real saves spanning two game versions.

## Using it

Open the web editor, drop your `.sav` file on it, change numbers, save.
On Chromium browsers the editor writes straight back to the file after a
permission prompt; elsewhere it downloads the edited file for you to move
back into the save folder.

Save locations:

| Platform | Path |
|---|---|
| Windows (Steam) | `%LOCALAPPDATA%\11bitstudios\TheAlters\Steam\Saved\SaveGames\<SteamID>\` |
| Steam Deck / Linux (Proton) | `~/.steam/steam/steamapps/compatdata/1601570/pfx/drive_c/users/steamuser/AppData/Local/11bitstudios/TheAlters/Steam/Saved/SaveGames/<SteamID>/` |

Close the game before editing, keep the automatic backup the page offers,
and consider pausing Steam Cloud sync for the game so it doesn't restore
the pre-edit file.

## What it can edit

- **Resources** (Metals, Rapidium, Organic Matter, Minerals, ...): stored
  amounts in base storage, with the load-screen preview kept in sync.
- **Item stack counts** (repair kits, rechargers, bridge pylons, ...).
- **Adding item types you don't own yet** - e.g. give yourself the four
  Bridge Pylons for the Act 1 lava-river bridge. Injection clones an
  existing stack record and lets the game rebuild the item from class
  defaults; this is verified in-game. Supported on current-version saves
  (archive v3); older v2 saves support count/resource edits only.

## Format notes (for the curious)

A world save is a plaintext metadata prefix followed by UE
`SerializeCompressed` zlib chunks (128 KiB blocks, tag `0x9E2A83C1`).
Two `i32` fields in the prefix store "bytes from here to end of file" and
must be rewritten whenever the payload length changes - miss the first one
and the game's save-scan thread crashes at startup with an access
violation (ask us how we know). The decompressed body is a stream of 11
bit's custom "Elb" object records: length-prefixed strings, tagged
properties, and sized spans that do not align with logical boundaries.
There are no checksums anywhere.

Module docs in `alters-save-core` carry the details:

- `sav`: outer framing, chunk headers, EOF-relative size fields
- `resources`: the `P9ResourceSubsystem` / `P9ResourceContainer` records
- `items`: the `P9ItemStack` list and the record-cloning injection
- `meta`: load-screen resource counts in the prefix

## Development

```
cargo test                             # unit + fixture tests
ALTERS_CORPUS_DIR=<your saves dir> cargo test   # full corpus verification
cargo run -p alters-save-cli -- corpus <dir>    # same checks, readable report
cargo run -p alters-save-cli -- show <file.sav>
wasm-pack build alters-save-web --target web --out-dir ../web/pkg
```

`test-data/` contains two fixture saves (audited to contain no personal
data): one archive v2, one v3. The verification battery
(`alters-save-core/src/verify.rs`) parses, roundtrips, edits, and injects
against every save it is given and asserts nothing else changed.

Not affiliated with 11 bit studios. Back up your saves; use at your own
risk.
