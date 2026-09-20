use ratatui::style::Color;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

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
    accent: Color::Rgb(0xcb, 0xa6, 0xf7),      // mauve
    accent_dim: Color::Rgb(0x6c, 0x70, 0x86),  // overlay0
    selected_bg: Color::Rgb(0x45, 0x47, 0x5a), // surface1
    // A little more saturated than the original washed-out #283b2e, but kept
    // restrained: the fill spans the whole row, so a strong colour covers a lot
    // of screen. Syntax-highlighted text on top stays well above WCAG AA.
    add_bg: Color::Rgb(0x2a, 0x43, 0x31),
    del_bg: Color::Rgb(0x4c, 0x2d, 0x34),
    placeholder: Color::Rgb(0x6c, 0x70, 0x86),
    hunk: Color::Rgb(0x89, 0xdc, 0xeb), // sky
    tick: Color::Rgb(0xa6, 0xe3, 0xa1), // green
    yellow: Color::Rgb(0xf9, 0xe2, 0xaf),
    red: Color::Rgb(0xf3, 0x8b, 0xa8),
    blue: Color::Rgb(0x89, 0xb4, 0xfa),
};

// Catppuccin Latte (light).
const LIGHT: Palette = Palette {
    accent: Color::Rgb(0x88, 0x39, 0xef),      // latte mauve
    accent_dim: Color::Rgb(0x8c, 0x8f, 0xa1),  // latte overlay0
    selected_bg: Color::Rgb(0xbc, 0xc0, 0xcc), // latte surface1
    // Restrained tints of latte green/red; light enough for dark text on top.
    add_bg: Color::Rgb(0xcd, 0xe5, 0xc6),
    del_bg: Color::Rgb(0xf4, 0xcd, 0xd2),
    placeholder: Color::Rgb(0x8c, 0x8f, 0xa1),
    hunk: Color::Rgb(0x04, 0xa5, 0xe5),   // latte sky
    tick: Color::Rgb(0x40, 0xa0, 0x2b),   // latte green
    yellow: Color::Rgb(0xdf, 0x8e, 0x1d), // latte yellow
    red: Color::Rgb(0xd2, 0x0f, 0x39),    // latte red
    blue: Color::Rgb(0x1e, 0x66, 0xf5),   // latte blue
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// The add/del fills must stay saturated enough to read as green and red
    /// rather than grey. The floor sits just above the original washed-out
    /// values (light add_bg #d6e9d0 was 0.11), leaving room to tune the fills
    /// up or down without the guard becoming a straitjacket.
    #[test]
    fn add_del_fills_are_saturated_enough_to_read_as_colour() {
        fn saturation(c: Color) -> f32 {
            let Color::Rgb(r, g, b) = c else {
                panic!("palette fills must be explicit rgb");
            };
            let (hi, lo) = (r.max(g).max(b), r.min(g).min(b));
            if hi == 0 {
                return 0.0;
            }
            (hi - lo) as f32 / hi as f32
        }
        for theme in [Theme::Dark, Theme::Light] {
            let p = Palette::for_theme(theme);
            for (name, c) in [("add_bg", p.add_bg), ("del_bg", p.del_bg)] {
                assert!(
                    saturation(c) > 0.13,
                    "{theme:?} {name} is too desaturated to read as a colour"
                );
            }
        }
    }

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
