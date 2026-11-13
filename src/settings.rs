//! Preferences kept between sessions, and where recovery files live.
//!
//! Stored as `clave=valor` lines in `ajustes.txt` inside the user's
//! configuration folder (`%APPDATA%\SharkPaint` on Windows).

use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    /// Colours the user saved.
    pub palette: Vec<[u8; 3]>,
    /// Colours used lately, most recent first.
    pub recent: Vec<[u8; 3]>,
    /// Tool identifier and the name of its shortcut key.
    pub tool_keys: Vec<(String, String)>,
}

/// SharkPaint's own folder for settings, created on demand.
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    }?;
    let dir = base.join("SharkPaint");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Where the drawing is saved automatically, to recover it after a crash.
pub fn recovery_path() -> Option<PathBuf> {
    Some(config_dir()?.join("recuperacion.spaint"))
}

fn colors_to_text(colors: &[[u8; 3]]) -> String {
    colors.iter().map(|c| format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect::<Vec<_>>().join(",")
}

fn colors_from_text(text: &str) -> Vec<[u8; 3]> {
    text.split(',')
        .filter_map(|hex| {
            let value = u32::from_str_radix(hex.trim(), 16).ok().filter(|_| hex.trim().len() == 6)?;
            Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
        })
        .collect()
}

impl Settings {
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "paleta={}\nrecientes={}\n",
            colors_to_text(&self.palette),
            colors_to_text(&self.recent)
        );
        for (tool, key) in &self.tool_keys {
            text.push_str(&format!("tecla.{tool}={key}\n"));
        }
        text
    }

    /// Reads settings, ignoring lines it does not understand.
    pub fn from_text(text: &str) -> Settings {
        let mut settings = Settings::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            match key.trim() {
                "paleta" => settings.palette = colors_from_text(value),
                "recientes" => settings.recent = colors_from_text(value),
                key => {
                    if let Some(tool) = key.strip_prefix("tecla.") {
                        settings.tool_keys.push((tool.to_owned(), value.trim().to_owned()));
                    }
                }
            }
        }
        settings
    }

    pub fn load() -> Settings {
        config_dir()
            .and_then(|dir| std::fs::read_to_string(dir.join("ajustes.txt")).ok())
            .map(|text| Settings::from_text(&text))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(dir) = config_dir() {
            let _ = std::fs::write(dir.join("ajustes.txt"), self.to_text());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_tolerate_junk() {
        let settings = Settings {
            palette: vec![[255, 0, 128], [1, 2, 3]],
            recent: vec![[0, 0, 0]],
            tool_keys: vec![("pincel".into(), "B".into()), ("borrador".into(), "X".into())],
        };
        assert_eq!(Settings::from_text(&settings.to_text()), settings);
        let messy = "paleta=ff0080,zzzzzz,12\nalgo raro\notra=cosa\ntecla.pincel= Q \n";
        let read = Settings::from_text(messy);
        assert_eq!(read.palette, vec![[255, 0, 128]]);
        assert_eq!(read.tool_keys, vec![("pincel".to_owned(), "Q".to_owned())]);
        assert!(Settings::from_text("").palette.is_empty());
    }
}
