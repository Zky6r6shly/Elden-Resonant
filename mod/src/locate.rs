//! Finding the player's CONTROL Resonant install.

use std::path::{Path, PathBuf};

pub const STEAM_APP_ID: &str = "3669870";

/// Values of every `"key" "value"` pair named `key` in a Valve KeyValues text.
pub fn vdf_values(text: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let toks = quoted(line.trim());
        if toks.len() == 2 && toks[0].eq_ignore_ascii_case(key) {
            out.push(toks[1].replace("\\\\", "\\"));
        }
    }
    out
}

fn quoted(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inside = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (inside, c) {
            (false, '"') => inside = true,
            (true, '"') => {
                out.push(std::mem::take(&mut cur));
                inside = false;
            }
            (true, '\\') => {
                cur.push('\\');
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (true, c) => cur.push(c),
            _ => {}
        }
    }
    out
}

/// Steam library folders listed by `steamapps/libraryfolders.vdf` under a Steam root.
pub fn steam_libraries(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    if let Ok(t) = std::fs::read_to_string(steam_root.join("steamapps").join("libraryfolders.vdf")) {
        for p in vdf_values(&t, "path") {
            let p = PathBuf::from(p);
            if !libs.contains(&p) {
                libs.push(p);
            }
        }
    }
    libs
}

/// The CONTROL Resonant folder inside one Steam library, if installed there.
pub fn in_steam_library(lib: &Path) -> Option<PathBuf> {
    let apps = lib.join("steamapps");
    if let Ok(t) = std::fs::read_to_string(apps.join(format!("appmanifest_{STEAM_APP_ID}.acf"))) {
        if let Some(dir) = vdf_values(&t, "installdir").into_iter().next() {
            let p = apps.join("common").join(dir);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    let rd = std::fs::read_dir(apps.join("common")).ok()?;
    rd.flatten().map(|e| e.path()).find(|p| {
        let n = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        p.is_dir() && n.contains("control") && n.contains("resonant")
    })
}

/// The CONTROL Resonant folder from Epic launcher manifests in `manifests_dir`.
pub fn in_epic_manifests(manifests_dir: &Path) -> Option<PathBuf> {
    let rd = std::fs::read_dir(manifests_dir).ok()?;
    for e in rd.flatten() {
        let p = e.path();
        if !p.extension().is_some_and(|x| x.eq_ignore_ascii_case("item")) {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) else { continue };
        let name = v["DisplayName"].as_str().unwrap_or_default().to_lowercase();
        if name.contains("control") && name.contains("resonant") {
            if let Some(loc) = v["InstallLocation"].as_str() {
                let loc = PathBuf::from(loc);
                if loc.is_dir() {
                    return Some(loc);
                }
            }
        }
    }
    None
}

/// Search in order: explicit path, Steam roots, Epic manifests. A folder only
/// counts when it has CONTROL Resonant's archives in it.
pub fn find(explicit: Option<PathBuf>, steam_roots: &[PathBuf], epic_manifests: &Path) -> Option<PathBuf> {
    let valid = |p: &Path| !crate::cotr::find_tocs(p, 4).is_empty();
    if let Some(p) = explicit {
        if valid(&p) {
            return Some(p);
        }
    }
    for root in steam_roots {
        for lib in steam_libraries(root) {
            if let Some(p) = in_steam_library(&lib) {
                if valid(&p) {
                    return Some(p);
                }
            }
        }
    }
    in_epic_manifests(epic_manifests).filter(|p| valid(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_steam_install_through_library_folders() {
        let base = std::env::temp_dir().join(format!("locate-test-{}", std::process::id()));
        let steam = base.join("Steam");
        let lib2 = base.join("Games");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let game = lib2.join("steamapps/common/CONTROL Resonant");
        std::fs::create_dir_all(game.join("data")).unwrap();
        std::fs::write(
            steam.join("steamapps/libraryfolders.vdf"),
            format!("\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", steam.display(), lib2.display()),
        )
        .unwrap();
        std::fs::write(lib2.join("steamapps/appmanifest_3669870.acf"), "\"AppState\"\n{\n\t\"appid\"\t\t\"3669870\"\n\t\"installdir\"\t\t\"CONTROL Resonant\"\n}\n").unwrap();
        // not valid until archives exist
        assert_eq!(find(None, &[steam.clone()], &base.join("none")), None);
        crate::cotr::tests::build(&game.join("data"), &[("a", "b.wem", b"x", false)]);
        assert_eq!(find(None, &[steam.clone()], &base.join("none")), Some(game.clone()));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn finds_epic_install() {
        let base = std::env::temp_dir().join(format!("locate-epic-{}", std::process::id()));
        let man = base.join("Manifests");
        let game = base.join("Epic Games/ControlResonant");
        std::fs::create_dir_all(&man).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        crate::cotr::tests::build(&game, &[("a", "b.wem", b"x", false)]);
        std::fs::write(man.join("X.item"), serde_json::json!({"DisplayName": "CONTROL Resonant", "InstallLocation": game}).to_string()).unwrap();
        assert_eq!(find(None, &[], &man), Some(game));
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn parses_escaped_windows_paths() {
        let v = vdf_values("\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"", "path");
        assert_eq!(v, vec!["D:\\SteamLibrary".to_string()]);
    }
}
