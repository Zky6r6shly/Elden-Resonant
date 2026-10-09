//! Picking which CONTROL Resonant sound plays for each cue.

use crate::generated::CUES;

/// For each cue, the index into `paths` of the sound it uses.
/// `pinned` holds per-cue exact paths from the ini, which win when present.
pub fn choose(paths: &[String], pinned: &[(String, String)]) -> Vec<Option<usize>> {
    let lower: Vec<String> = paths.iter().map(|p| p.to_lowercase()).collect();
    let wems: Vec<usize> = (0..paths.len()).filter(|&i| lower[i].ends_with(".wem")).collect();
    CUES.iter()
        .map(|cue| {
            if let Some((_, want)) = pinned.iter().find(|(k, _)| k == cue.key) {
                let want = want.to_lowercase().replace('\\', "/");
                if let Some(&i) = wems.iter().find(|&&i| lower[i] == want) {
                    return Some(i);
                }
            }
            for kw in cue.keywords {
                // first match in a stable (sorted) order so the choice is the same every run
                if let Some(&i) = wems.iter().filter(|&&i| lower[i].contains(kw)).min_by(|&&a, &&b| lower[a].cmp(&lower[b])) {
                    return Some(i);
                }
            }
            None
        })
        .collect()
}

/// Which ability cue an FP drop of `delta` belongs to, if any.
pub fn ability_for_fp_drop(delta: i32) -> Option<usize> {
    crate::generated::ABILITIES.iter().position(|a| a.fp_cost as i32 == delta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generated::CUE_ARRIVAL;

    #[test]
    fn keywords_and_pins() {
        let paths: Vec<String> = ["sfx/abilities/push_force_01.wem", "music/mus_combat.wem", "music/main_theme.wem", "ui/click.wem", "x/aberrant_morph.wem", "readme.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let c = choose(&paths, &[]);
        assert_eq!(c[CUE_ARRIVAL], Some(2)); // "music/main" matches music/main_theme.wem
        let push = CUES.iter().position(|c| c.key == "ability_push").unwrap();
        assert_eq!(c[push], Some(0));
        let pinned = vec![("arrival".to_string(), "UI\\click.wem".to_string())];
        assert_eq!(choose(&paths, &pinned)[CUE_ARRIVAL], Some(3));
    }

    #[test]
    fn fp_costs_map_back() {
        for (i, a) in crate::generated::ABILITIES.iter().enumerate() {
            assert_eq!(ability_for_fp_drop(a.fp_cost as i32), Some(i));
        }
        assert_eq!(ability_for_fp_drop(3), None);
    }
}
