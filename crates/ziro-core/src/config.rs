use serde::{Deserialize, Serialize};
use std::{fs, path::Path};
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub menu: Menu,
    pub ghost: Ghost,
    pub keys: Keys,
    pub worker: Worker,
    pub history: History,
    pub theme: Theme,
}
macro_rules! config_section {
    ($name:ident { $($field:ident : $ty:ty = $value:expr),* $(,)? }) => {
        #[derive(Debug, Clone, Deserialize, Serialize)]
        #[serde(default)]
        pub struct $name { $(pub $field: $ty),* }
        impl Default for $name { fn default() -> Self { Self { $($field: $value),* } } }
    }
}
config_section!(Menu {
    max_items: usize = 8,
    max_width: usize = 48,
    min_prefix_first_word: usize = 1,
    show_descriptions: bool = true,
    border: String = "rounded".into(),
    icons: String = "emoji".into(),
    enter_accepts: String = "explicit".into()
});
config_section!(Ghost {
    enable: bool = true
});
config_section!(Keys { accept: String = "^I".into(), arrows: String = "menu".into(), enter_menu: String = "^@".into(), next: Vec<String> = vec!["^[[B".into(), "^[OB".into()], prev: Vec<String> = vec!["^[[A".into(), "^[OA".into(), "^[[Z".into()], dismiss: String = "^[".into() });
config_section!(Worker {
    timeout_ms: u64 = 3000,
    refresh_idle_ms: u64 = 80
});
config_section!(History {
    import_histfile: bool = true,
    cwd_boost: f64 = 1.5,
    failed_penalty: f64 = 0.3
});
config_section!(Theme {
    selected: String = "bg=4,fg=15".into(),
    r#match: String = "fg=3,bold".into(),
    description: String = "fg=8".into(),
    border: String = "fg=4".into(),
    ghost: String = "fg=8".into()
});
impl Config {
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), Box<dyn std::error::Error>> {
        let text = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Self::default(), vec![]));
            }
            Err(e) => return Err(e.into()),
        };
        let value: toml::Value = toml::from_str(&text)?;
        let defaults: toml::Value = toml::Value::try_from(Self::default())?;
        let mut warnings = vec![];
        if let Some(table) = value.as_table() {
            for (section, fields) in table {
                if let Some(expected) = defaults.get(section) {
                    if let Some(fields) = fields.as_table() {
                        for key in fields.keys() {
                            if expected.get(key).is_none() {
                                warnings.push(format!("unknown key {section}.{key}"));
                            }
                        }
                    }
                } else {
                    warnings.push(format!("unknown key {section}"));
                }
            }
        }
        let mut config: Self = value.try_into()?;
        config.menu.max_items = config.menu.max_items.clamp(1, 32);
        config.worker.timeout_ms = config.worker.timeout_ms.max(1);
        Ok((config, warnings))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_defaults_and_unknown_keys() {
        let path = std::env::temp_dir().join(format!("ziro-config-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert_eq!(Config::load(&path).unwrap().0.menu.max_items, 8);
        std::fs::write(
            &path,
            "[menu]\nmax_items=999\nfuture=true\n[ghost]\nenable=false\n",
        )
        .unwrap();
        let (c, w) = Config::load(&path).unwrap();
        assert_eq!(c.menu.max_items, 32);
        assert!(!c.ghost.enable);
        assert_eq!(w, vec!["unknown key menu.future"]);
        std::fs::remove_file(path).unwrap();
    }
}
