# Elden Resonant

**Elden Ring × CONTROL Resonant.** Play the Lands Between as a Resonant: Dylan Faden's shapeshifting Aberrant in your hands, his Resonance powers on your belt, and CONTROL Resonant's own sounds, read from your copy of the game, every time you use them.

- **Host game:** Elden Ring. Melty installs into it and starts it through ModEngine2, offline with Easy Anti-Cheat off.
- **Also uses:** CONTROL Resonant (Steam app 3669870, or Epic). The mod finds your install by itself and reads its sounds at run time. No CONTROL Resonant files ship with the mod. Without it, the game still runs and the Aberrant's descriptions say what's missing.
- **Players:** solo.

## What you get

Make a new character and pick the **Resonant** class (it replaces the Wretch). You start with:

| Aberrant form | Hand | Combo finisher (its skill) | Elden Ring moveset it borrows |
|---|---|---|---|
| Flurry: twin blades | right | Split Pivot | Misericorde / Double Slash |
| Slash: wide scythe | right | Spin Edge | Grave Scythe / Spinning Strikes |
| Slice: single heavy edge | right | Wide Edge | Nagakiba / Spinning Slash |
| Crush: huge mallet | left | Concussive Beatdown | Brick Hammer / Wild Strikes |
| Drill: boring point | left | Uppercut Surge | Estoc / Giant Hunt |
| Extend: chain staff | left | Split Orbit | Urumi / Spinning Weapon |

Seven **Resonance powers** sit on your item belt. They never run out and cost FP, which stands in for Resonance: Force Push, Offensive Shield, Ignite, Inferno, Growth, Barrage and Remote Explosion.

CONTROL Resonant sounds play when you enter the world, shift to an Aberrant form, land a finisher or use a power. The mod lists every `.wem` in your CONTROL Resonant archives, picks one per cue by keyword (see `sheets/cues.json`), decodes it with the bundled vgmstream and caches it in `%LOCALAPPDATA%\EldenResonant\cache`. The log next to the cache says which file each cue uses. You can pin your own choices in `elden-resonant.ini`.

**Traversal powers**, always on for every character, all on the jump button:

- **Double Jump:** press jump again in mid-air.
- **Elevate:** keep holding jump in mid-air to float and glide slowly down. It drains FP while held, and gliding doesn't count as a fall.
- **Rush:** jump while sprinting to turn the jump into a long, fast forward leap.

They work by scaling Elden Ring's own gravity and movement on your character, so walls and floors still stop you. They're off while riding Torrent.

**Free powers:** set `free_powers=true` in `elden-resonant.ini`, or press **F8** in game, and powers, skills and traversal cost no FP.

Not included: Shift, which rotates CONTROL Resonant's own levels and has no equivalent in Elden Ring's maps.

## How it's built

- `sheets/*.json` are the design and the source of truth: one row per form, finisher, power, class, sound cue and game hook.
- `tools/preflight.py` overlays the sheets and lists every unfilled cell and every reference that doesn't resolve. With `--paramdex <Paramdex/ER/Names>`, it also checks each Elden Ring row id against the community param names.
- `tools/gen.py` turns the sheets into `mod/src/generated.rs`, one Rust value per row.
- `mod/` is the ModEngine2 DLL (Rust, built on [fromsoftware-rs](https://github.com/vswarte/fromsoftware-rs)):
  - `win.rs` changes the weapon, goods and class params at boot and points the weapon, power, skill and class text at the mod's own strings in the game's loaded text tables. A per-frame hook watches FP and the right-hand weapon to trigger sounds.
  - `cotr.rs` is a reader for CONTROL Resonant's `.rmdtoc`/`.rmdblob` archives (COTR v3, LZ4 chunks).
  - `locate.rs` finds CONTROL Resonant through `ELDEN_RESONANT_CR_PATH`, `cr_path` in the ini, Steam's library folders or the Epic launcher's manifests.
- `tools/package.sh` runs the preflight, codegen, tests and cross-compile, then zips the DLL, vgmstream and the licenses into `dist/`.

Build on Linux with `rustup target add x86_64-pc-windows-gnu`, `mingw-w64` and `./tools/package.sh`.

## Status

| Check | Result |
|---|---|
| Sheet preflight (327 cells, Elden Ring ids against Paramdex) | clean |
| Unit tests (archive reader, text tables, install finder, cues, traversal, ini) | 17 pass |
| Windows DLL cross-compile | builds, imports only system DLLs |
| Under Wine with a stand-in CONTROL Resonant archive | DLL loads, finds the install, indexes the archive, picks cues, decodes with vgmstream to WAV |
| Under Wine with stand-in Elden Ring text tables | weapon, power and class names and descriptions replaced, missing-game notice shown |
| **In Elden Ring with CONTROL Resonant installed** | **not tested yet**: needs a Windows PC with both games |

The per-frame hook (sounds, traversal, free powers) uses addresses for Elden Ring 2.7.1.0 (JP 2.7.1.1). On other versions the class, items and text still change, but those features stay off and the log says so. The traversal numbers in `sheets/traversal.json` are starting values to tune in game.

## Credits

- Elden Ring © FromSoftware / Bandai Namco. CONTROL Resonant © Remedy Entertainment. Both games' content is read from the player's own installs.
- [fromsoftware-rs](https://github.com/vswarte/fromsoftware-rs) (MIT/Apache-2.0) by vswarte and contributors.
- [vgmstream](https://github.com/vgmstream/vgmstream) (ISC-style), bundled to decode Wwise audio.
- [Paramdex](https://github.com/soulsmods/Paramdex) param names, used to check row ids at design time.
- The COTR archive layout was learned from UmFzbXVz's [Control-Resonant-asset-conversion-kit](https://github.com/UmFzbXVz/Control-Resonant-asset-conversion-kit).
