//! `elden-resonant.ini`: optional overrides, one `key=value` per line.
//!
//! cr_path=D:\Games\CONTROL Resonant   (where CONTROL Resonant is installed)
//! volume=0.8                           (master volume, 0..1)
//! cue.arrival=audio/music/xyz.wem      (pin a cue to an exact archive path)

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub cr_path: Option<String>,
    pub volume: Option<f32>,
    pub pinned: Vec<(String, String)>,
}

pub fn parse(text: &str) -> Config {
    let mut c = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim(), v.trim());
        match k {
            "cr_path" if !v.is_empty() => c.cr_path = Some(v.to_string()),
            "volume" => c.volume = v.parse().ok().map(|x: f32| x.clamp(0.0, 1.0)),
            _ if k.starts_with("cue.") && !v.is_empty() => c.pinned.push((k[4..].to_string(), v.to_string())),
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let c = super::parse("; hi\n[x]\ncr_path = D:\\CR\nvolume=3\ncue.arrival=a/b.wem\njunk\n");
        assert_eq!(c.cr_path.as_deref(), Some("D:\\CR"));
        assert_eq!(c.volume, Some(1.0));
        assert_eq!(c.pinned, vec![("arrival".into(), "a/b.wem".into())]);
    }
}
