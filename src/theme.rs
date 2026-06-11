use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme { Dark, Light }

impl Default for Theme { fn default() -> Self { Theme::Dark } }

/// All UI colors for one theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub accent: Color,
    pub accent_dim: Color,
    pub selected_bg: Color,
    pub add_bg: Color,
    pub del_bg: Color,
    pub placeholder: Color,
    pub hunk: Color,
    pub tick: Color,
    pub yellow: Color,
    pub red: Color,
    pub blue: Color,
}

impl Palette {
    pub fn for_theme(theme: Theme) -> Palette {
        match theme {
            Theme::Dark => DARK,
            Theme::Light => LIGHT,
        }
    }
}

// Catppuccin Mocha (dark) — current values.
const DARK: Palette = Palette {
    accent: Color::Rgb(0xcb, 0xa6, 0xf7),       // mauve
    accent_dim: Color::Rgb(0x6c, 0x70, 0x86),   // overlay0
    selected_bg: Color::Rgb(0x45, 0x47, 0x5a),  // surface1
    add_bg: Color::Rgb(0x28, 0x3b, 0x2e),
    del_bg: Color::Rgb(0x44, 0x2b, 0x30),
    placeholder: Color::Rgb(0x6c, 0x70, 0x86),
    hunk: Color::Rgb(0x89, 0xdc, 0xeb),         // sky
    tick: Color::Rgb(0xa6, 0xe3, 0xa1),         // green
    yellow: Color::Rgb(0xf9, 0xe2, 0xaf),
    red: Color::Rgb(0xf3, 0x8b, 0xa8),
    blue: Color::Rgb(0x89, 0xb4, 0xfa),
};

// Catppuccin Latte (light).
const LIGHT: Palette = Palette {
    accent: Color::Rgb(0x88, 0x39, 0xef),       // latte mauve
    accent_dim: Color::Rgb(0x8c, 0x8f, 0xa1),   // latte overlay0
    selected_bg: Color::Rgb(0xbc, 0xc0, 0xcc),  // latte surface1
    add_bg: Color::Rgb(0xd6, 0xe9, 0xd0),       // light green tint (latte green #40a02b)
    del_bg: Color::Rgb(0xf2, 0xd5, 0xd9),       // light red tint (latte red #d20f39)
    placeholder: Color::Rgb(0x8c, 0x8f, 0xa1),
    hunk: Color::Rgb(0x04, 0xa5, 0xe5),         // latte sky
    tick: Color::Rgb(0x40, 0xa0, 0x2b),         // latte green
    yellow: Color::Rgb(0xdf, 0x8e, 0x1d),       // latte yellow
    red: Color::Rgb(0xd2, 0x0f, 0x39),          // latte red
    blue: Color::Rgb(0x1e, 0x66, 0xf5),         // latte blue
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_palette_add_bg_differs_from_del_bg() {
        let dark = Palette::for_theme(Theme::Dark);
        assert_ne!(dark.add_bg, dark.del_bg);
    }

    #[test]
    fn dark_palette_accent_differs_from_accent_dim() {
        let dark = Palette::for_theme(Theme::Dark);
        assert_ne!(dark.accent, dark.accent_dim);
    }

    #[test]
    fn dark_and_light_palettes_differ() {
        let dark = Palette::for_theme(Theme::Dark);
        let light = Palette::for_theme(Theme::Light);
        // Accent colors differ between themes
        assert_ne!(dark.accent, light.accent);
        // add_bg colors differ (dark is dark tint, light is light tint)
        assert_ne!(dark.add_bg, light.add_bg);
    }

    #[test]
    fn theme_default_is_dark() {
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn for_theme_returns_correct_palette() {
        // Dark palette accent should be Mocha mauve
        let dark = Palette::for_theme(Theme::Dark);
        assert_eq!(dark.accent, Color::Rgb(0xcb, 0xa6, 0xf7));
        // Light palette accent should be Latte mauve
        let light = Palette::for_theme(Theme::Light);
        assert_eq!(light.accent, Color::Rgb(0x88, 0x39, 0xef));
    }
}
