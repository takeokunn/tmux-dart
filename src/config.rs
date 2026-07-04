use std::env;

use crate::{
    jump::{KeyPosition, LabelKeys, MatchMode},
    overlay::{OverlayStyle, OverlayTheme, decode_tmux_color},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpOptions {
    pub label_keys: LabelKeys,
    pub match_mode: MatchMode,
    pub case_sensitive: bool,
    pub auto_jump: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JumpConfig {
    pub options: JumpOptions,
    pub overlay_style: OverlayStyle,
}

impl JumpConfig {
    pub fn from_env() -> Self {
        Self::from_values(EnvValues {
            label_keys: env::var("JUMP_LABEL_KEYS").ok(),
            match_mode: env::var("JUMP_MATCH_MODE").ok(),
            case_sensitive: env::var("JUMP_CASE_SENSITIVE").ok(),
            auto_jump: env::var("JUMP_AUTO_JUMP").ok(),
            theme: env::var("JUMP_THEME").ok(),
            background_color: env::var("JUMP_BACKGROUND_COLOR").ok(),
            foreground_color: env::var("JUMP_FOREGROUND_COLOR").ok(),
            key_position: env::var("JUMP_KEYS_POSITION").ok(),
        })
    }

    pub fn from_values(values: EnvValues) -> Self {
        let options = JumpOptions {
            label_keys: values
                .label_keys
                .as_deref()
                .map(LabelKeys::from_env)
                .unwrap_or_default(),
            match_mode: MatchMode::from_env(values.match_mode.as_deref().unwrap_or("word")),
            case_sensitive: option_enabled(values.case_sensitive.as_deref().unwrap_or_default()),
            auto_jump: !option_disabled(values.auto_jump.as_deref().unwrap_or_default()),
        };
        let mut overlay_style =
            OverlayTheme::from_env(values.theme.as_deref().unwrap_or("classic")).defaults();
        if let Some(background_color) = values.background_color.as_deref() {
            overlay_style.background = decode_tmux_color(background_color);
        }
        if let Some(foreground_color) = values.foreground_color.as_deref() {
            overlay_style.foreground = decode_tmux_color(foreground_color);
        }
        overlay_style.key_position =
            KeyPosition::from_env(values.key_position.as_deref().unwrap_or("left"));

        Self {
            options,
            overlay_style,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvValues {
    pub label_keys: Option<String>,
    pub match_mode: Option<String>,
    pub case_sensitive: Option<String>,
    pub auto_jump: Option<String>,
    pub theme: Option<String>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub key_position: Option<String>,
}

fn option_enabled(value: &str) -> bool {
    value == "1" || value == "on" || value == "true" || value == "yes"
}

fn option_disabled(value: &str) -> bool {
    value == "0" || value == "off" || value == "false" || value == "no"
}

#[cfg(test)]
mod tests {
    use super::{EnvValues, JumpConfig};
    use crate::jump::{KeyPosition, MatchMode};

    #[test]
    fn config_defaults_match_tmux_jump_style_behavior() {
        let config = JumpConfig::from_values(EnvValues::default());

        assert_eq!(config.options.match_mode, MatchMode::Word);
        assert!(!config.options.case_sensitive);
        assert!(config.options.auto_jump);
        assert_eq!(config.overlay_style.key_position, KeyPosition::Left);
        assert_eq!(config.overlay_style.background, "\u{1b}[0m\u{1b}[32m");
        assert_eq!(config.overlay_style.foreground, "\u{1b}[1m\u{1b}[31m");
        assert_eq!(config.overlay_style.label_style, "\u{1b}[1m");
    }

    #[test]
    fn config_parses_custom_values() {
        let config = JumpConfig::from_values(EnvValues {
            label_keys: Some(String::from("abc")),
            match_mode: Some(String::from("char")),
            case_sensitive: Some(String::from("on")),
            auto_jump: Some(String::from("off")),
            theme: Some(String::from("contrast")),
            background_color: Some(String::from(r#"\e[34m"#)),
            foreground_color: Some(String::from(r#"\e[35m"#)),
            key_position: Some(String::from("right")),
        });

        assert_eq!(config.options.label_keys.as_slice(), ['a', 'b', 'c']);
        assert_eq!(config.options.match_mode, MatchMode::Char);
        assert!(config.options.case_sensitive);
        assert!(!config.options.auto_jump);
        assert_eq!(config.overlay_style.key_position, KeyPosition::Right);
        assert_eq!(config.overlay_style.background, "\u{1b}[34m");
        assert_eq!(config.overlay_style.foreground, "\u{1b}[35m");
        assert_eq!(config.overlay_style.label_style, "\u{1b}[1m\u{1b}[7m");
    }
}
