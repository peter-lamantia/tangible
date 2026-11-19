use crate::theme::ThemePreference;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub struct Preferences {
    pub theme: ThemePreference,
    pub orthographic: bool,
    pub width: u32,
    pub height: u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            orthographic: false,
            width: 1280,
            height: 720,
        }
    }
}
impl Preferences {
    fn path() -> Option<PathBuf> {
        let root = if cfg!(target_os = "windows") {
            std::env::var_os("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Application Support"))
        } else {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        }?;
        Some(root.join("tangible/preferences"))
    }
    pub fn load() -> Self {
        let mut prefs = Self::default();
        if let Some(text) = Self::path().and_then(|p| std::fs::read_to_string(p).ok()) {
            let fields: Vec<_> = text.split_whitespace().collect();
            if fields.len() == 4 {
                prefs.theme = match fields[0] {
                    "dark" => ThemePreference::Dark,
                    "light" => ThemePreference::Light,
                    _ => ThemePreference::System,
                };
                prefs.orthographic = fields[1] == "true";
                prefs.width = fields[2].parse::<u32>().unwrap_or(1280).clamp(320, 16384);
                prefs.height = fields[3].parse::<u32>().unwrap_or(720).clamp(240, 16384);
            }
        }
        prefs
    }
    pub fn save(self) {
        let Some(path) = Self::path() else { return };
        let theme = match self.theme {
            ThemePreference::System => "system",
            ThemePreference::Dark => "dark",
            ThemePreference::Light => "light",
        };
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(
                path,
                format!(
                    "{theme} {} {} {}\n",
                    self.orthographic, self.width, self.height
                ),
            )
        })();
        if let Err(error) = result {
            log::warn!("Could not save preferences: {error}");
        }
    }
}
