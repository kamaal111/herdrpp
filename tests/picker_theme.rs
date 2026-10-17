use herdrpp::picker::theme::{
    Palette, THEME_NAMES, Theme, canonical_theme_name, config_path, try_parse_color,
};
use ratatui::style::Color;
use std::path::PathBuf;
use terminal_input::event::ThemeMode;

#[test]
fn all_builtins_resolve_to_their_configured_palette() {
    for name in THEME_NAMES {
        let theme = Theme::parse(&format!("[theme]\nname = {name:?}")).unwrap();
        assert_eq!(Some(theme.palette()), Palette::from_name(name));
    }
}

#[test]
fn aliases_and_normalization_resolve_like_herdr() {
    for (alias, name) in [
        ("catppuccin-mocha", "catppuccin"),
        ("light", "catppuccin-latte"),
        ("Tokyo_Night", "tokyo-night"),
        ("tokyo-day", "tokyo-night-day"),
        ("gruvbox-dark", "gruvbox"),
        ("onedark", "one-dark"),
        ("onelight", "one-light"),
        ("solarized-dark", "solarized"),
        ("lotus", "kanagawa-lotus"),
        ("rosepine", "rose-pine"),
        ("dawn", "rose-pine-dawn"),
        ("rosepine-dawn", "rose-pine-dawn"),
    ] {
        assert_eq!(canonical_theme_name(alias), Some(name));
        assert_eq!(Palette::from_name(alias), Palette::from_name(name));
    }
}

#[test]
fn dracula_matches_herdr_colors() {
    let palette = Theme::parse("[theme]\nname = 'dracula'").unwrap().palette();
    assert_eq!(palette.panel_bg, Color::Rgb(40, 42, 54));
    assert_eq!(palette.text, Color::Rgb(248, 248, 242));
    assert_eq!(palette.accent, Color::Rgb(189, 147, 249));
    assert_eq!(palette.surface0, Color::Rgb(68, 71, 90));
}

#[test]
fn config_path_precedence_matches_herdr() {
    assert_eq!(
        config_path(
            Some("/custom/herdr.toml".into()),
            Some("/xdg".into()),
            Some("/home".into())
        ),
        PathBuf::from("/custom/herdr.toml")
    );
    assert_eq!(
        config_path(None, Some("/xdg".into()), Some("/home".into())),
        PathBuf::from("/xdg/herdr/config.toml")
    );
    assert_eq!(
        config_path(None, None, Some("/home".into())),
        PathBuf::from("/home/.config/herdr/config.toml")
    );
}

#[test]
fn unrelated_sections_do_not_affect_theme_parsing() {
    let theme = Theme::parse("[ui]\nstatus_indicators = 'symbols'\n[future]\nsetting = true\n[theme]\nname = 'dracula'\nfuture = true").unwrap();
    assert_eq!(theme.palette(), Palette::dracula());
}

#[test]
fn shared_and_mode_overrides_layer_after_selected_base() {
    let mut theme = Theme::parse(
        r##"
[theme]
name = "nord"
auto_switch = true
dark_name = "dracula"
light_name = "one-light"
[theme.custom]
accent = "#123"
text = "rgb(1, 2, 3)"
[theme.custom.light]
accent = "#abcdef"
[theme.custom.dark]
panel_bg = "#010203"
"##,
    )
    .unwrap();
    assert_eq!(theme.palette().accent, Color::Rgb(17, 34, 51));
    assert_eq!(theme.palette().panel_bg, Color::Rgb(1, 2, 3));
    theme.set_appearance(ThemeMode::Light);
    assert_eq!(theme.palette().accent, Color::Rgb(171, 205, 239));
    assert_eq!(theme.palette().text, Color::Rgb(1, 2, 3));
    assert_eq!(theme.palette().panel_bg, Palette::one_light().panel_bg);
    theme.set_appearance(ThemeMode::Dark);
    assert_eq!(theme.palette().panel_bg, Color::Rgb(1, 2, 3));
}

#[test]
fn auto_switch_uses_sibling_themes_and_defaults_to_dark() {
    let mut theme = Theme::parse("[theme]\nname = 'tokyo-night-day'\nauto_switch = true").unwrap();
    assert_eq!(theme.palette(), Palette::tokyo_night());
    theme.set_appearance(ThemeMode::Light);
    assert_eq!(theme.palette(), Palette::tokyo_night_day());
}

#[test]
fn appearance_reports_do_not_change_manual_themes() {
    let mut theme =
        Theme::parse("[theme]\nname = 'dracula'\n[theme.custom.light]\naccent = 'red'").unwrap();
    theme.set_appearance(ThemeMode::Light);
    assert_eq!(theme.palette(), Palette::dracula());
}

#[test]
fn legacy_accent_yields_to_custom_accent() {
    assert_eq!(
        Theme::parse("[ui]\naccent = 'red'")
            .unwrap()
            .palette()
            .accent,
        Color::Red
    );
    assert_eq!(
        Theme::parse("[ui]\naccent = 'cyan'")
            .unwrap()
            .palette()
            .accent,
        Palette::catppuccin().accent
    );
    assert_eq!(
        Theme::parse("[ui]\naccent = 'red'\n[theme.custom]\naccent = 'green'")
            .unwrap()
            .palette()
            .accent,
        Color::Green
    );
}

#[test]
fn unknown_names_and_colors_use_herdr_fallbacks() {
    assert_eq!(
        Theme::parse("[theme]\nname = 'missing'").unwrap().palette(),
        Palette::catppuccin()
    );
    let mut theme = Theme::parse("[theme]\nauto_switch = true\nlight_name = 'missing'").unwrap();
    theme.set_appearance(ThemeMode::Light);
    assert_eq!(theme.palette(), Palette::catppuccin_latte());
    assert_eq!(
        Theme::parse("[theme.custom]\naccent = 'missing'")
            .unwrap()
            .palette()
            .accent,
        Color::Cyan
    );
    assert!(try_parse_color("#💥ab").is_err());
    assert!(Theme::parse("[theme]\nauto_switch = 'yes'").is_err());
}

#[test]
fn color_syntax_and_terminal_reset_match_herdr() {
    assert_eq!(try_parse_color("#123").unwrap(), Color::Rgb(17, 34, 51));
    assert_eq!(try_parse_color("#123456").unwrap(), Color::Rgb(18, 52, 86));
    assert_eq!(
        try_parse_color("rgb(1, 2, 3)").unwrap(),
        Color::Rgb(1, 2, 3)
    );
    for reset in ["reset", "default", "none", "transparent"] {
        assert_eq!(try_parse_color(reset).unwrap(), Color::Reset);
    }
    assert_eq!(
        Theme::parse("[theme]\nname = 'terminal'")
            .unwrap()
            .palette()
            .panel_bg,
        Color::Reset
    );
}

struct ThemeFile {
    directory: PathBuf,
    path: PathBuf,
}

impl ThemeFile {
    fn new(text: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "picker-theme-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("config.toml");
        std::fs::write(&path, text).unwrap();
        Self { directory, path }
    }
}

impl Drop for ThemeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn missing_config_uses_default_without_diagnostic() {
    let fixture = ThemeFile::new("");
    let (theme, diagnostics) = Theme::load_path(&fixture.directory.join("missing.toml"));
    assert_eq!(theme.palette(), Palette::catppuccin());
    assert!(diagnostics.is_empty());
}

#[test]
fn malformed_theme_and_unreadable_file_fall_back_with_diagnostics() {
    let fixture = ThemeFile::new("[theme]\nauto_switch = 'yes'");
    let (theme, diagnostics) = Theme::load_path(&fixture.path);
    assert_eq!(theme.palette(), Palette::catppuccin());
    assert_eq!(diagnostics.len(), 1);
    assert!(diagnostics[0].contains("invalid Herdr theme"));
    let (theme, diagnostics) = Theme::load_path(&fixture.directory);
    assert_eq!(theme.palette(), Palette::catppuccin());
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn invalid_tokens_are_diagnosed_without_discarding_valid_overrides() {
    let fixture = ThemeFile::new(
        "[theme]\nname = 'missing'\n[theme.custom]\naccent = 'missing'\ntext = '#123'",
    );
    let (theme, diagnostics) = Theme::load_path(&fixture.path);
    assert_eq!(theme.palette().accent, Color::Cyan);
    assert_eq!(theme.palette().text, Color::Rgb(17, 34, 51));
    assert_eq!(diagnostics.len(), 2);
}

#[test]
fn saved_config_is_reloaded_on_next_invocation() {
    let fixture = ThemeFile::new("\u{feff}[theme]\nname = 'dracula'");
    assert_eq!(
        Theme::load_path(&fixture.path).0.palette(),
        Palette::dracula()
    );
    std::fs::write(&fixture.path, "[theme]\nname = 'one-light'").unwrap();
    assert_eq!(
        Theme::load_path(&fixture.path).0.palette(),
        Palette::one_light()
    );
}
