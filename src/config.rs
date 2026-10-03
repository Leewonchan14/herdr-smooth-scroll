//! Settings loaded from `$HERDR_PLUGIN_CONFIG_DIR/config.toml`.
//!
//! The option names and semantics mirror `azorng/tmux-smooth-scroll`'s `@smooth-scroll-*`
//! options: `speed` is a 0-1000 scale where lower is faster, and `easing` selects the per-step
//! velocity curve.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Upper bound of the `speed` scale; higher values are clamped.
pub const SPEED_MAX: u32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Easing {
    Linear,
    #[default]
    Sine,
    Quad,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 0-1000; lower is faster (the base per-line delay is `1000 + speed * 90` microseconds).
    pub speed: u32,
    pub easing: Easing,
    /// Lines per `scroll-up` / `scroll-down`.
    pub normal_lines: usize,
    /// Lines per `halfpage-*`; `None` means half the pane height.
    pub halfpage_lines: Option<usize>,
    /// Lines per `page-*`; `None` means the full pane height.
    pub fullpage_lines: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            speed: 100,
            easing: Easing::Sine,
            normal_lines: 3,
            halfpage_lines: None,
            fullpage_lines: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawConfig {
    speed: Option<u32>,
    easing: Option<Easing>,
    normal_lines: Option<usize>,
    halfpage_lines: Option<usize>,
    fullpage_lines: Option<usize>,
}

/// Load settings from `<config_dir>/config.toml`. A missing file yields the defaults.
pub fn load(config_dir: Option<&Path>) -> Result<Settings> {
    let Some(dir) = config_dir else {
        return Ok(Settings::default());
    };
    let path = dir.join("config.toml");
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let raw: RawConfig =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(compile(raw))
}

fn compile(raw: RawConfig) -> Settings {
    let defaults = Settings::default();
    Settings {
        speed: raw.speed.unwrap_or(defaults.speed).min(SPEED_MAX),
        easing: raw.easing.unwrap_or(defaults.easing),
        normal_lines: raw.normal_lines.unwrap_or(defaults.normal_lines).max(1),
        halfpage_lines: raw.halfpage_lines.map(|lines| lines.max(1)),
        fullpage_lines: raw.fullpage_lines.map(|lines| lines.max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn defaults_match_the_tmux_plugin() {
        let settings = Settings::default();
        assert_eq!(settings.speed, 100);
        assert_eq!(settings.easing, Easing::Sine);
        assert_eq!(settings.normal_lines, 3);
        assert_eq!(settings.halfpage_lines, None);
        assert_eq!(settings.fullpage_lines, None);
    }

    #[test]
    fn config_overrides_speed_easing_and_lines() {
        let settings = compile(
            toml::from_str(
                r#"
                speed = 40
                easing = "quad"
                normal_lines = 5
                halfpage_lines = 12
                fullpage_lines = 30
                "#,
            )
            .unwrap(),
        );
        assert_eq!(settings.speed, 40);
        assert_eq!(settings.easing, Easing::Quad);
        assert_eq!(settings.normal_lines, 5);
        assert_eq!(settings.halfpage_lines, Some(12));
        assert_eq!(settings.fullpage_lines, Some(30));
    }

    #[test]
    fn speed_above_the_scale_is_clamped() {
        let settings = compile(toml::from_str("speed = 4000").unwrap());
        assert_eq!(settings.speed, SPEED_MAX);
    }

    #[test]
    fn line_counts_never_drop_to_zero() {
        let settings = compile(toml::from_str("normal_lines = 0\nhalfpage_lines = 0").unwrap());
        assert_eq!(settings.normal_lines, 1);
        assert_eq!(settings.halfpage_lines, Some(1));
    }

    #[test]
    fn a_bad_easing_value_is_an_error() {
        let raw = toml::from_str::<RawConfig>("easing = \"bounce\"").unwrap_err();
        assert!(raw.to_string().contains("bounce"), "{raw}");
    }

    #[test]
    fn a_missing_file_yields_the_defaults() {
        let dir = std::env::temp_dir().join(format!("herdr-smooth-scroll-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(load(Some(&dir)).unwrap(), Settings::default());
        let _ = std::fs::remove_dir(&dir);
    }
}
