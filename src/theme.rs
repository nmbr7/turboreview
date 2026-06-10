use ratatui::style::Color;

// Catppuccin Mocha palette — all UI colors are defined here.

/// Mauve — focused border, hunk headers.
pub const ACCENT: Color = Color::Rgb(0xcb, 0xa6, 0xf7);

/// Overlay0 — unfocused border, gutter, status text.
pub const ACCENT_DIM: Color = Color::Rgb(0x6c, 0x70, 0x86);

/// Surface1 — selected row background.
pub const SELECTED_BG: Color = Color::Rgb(0x45, 0x47, 0x5a);

/// Dark green tint (derived from Mocha green #a6e3a1) — added line background.
pub const ADD_BG: Color = Color::Rgb(0x28, 0x3b, 0x2e);

/// Dark red tint (derived from Mocha red #f38ba8) — deleted line background.
pub const DEL_BG: Color = Color::Rgb(0x44, 0x2b, 0x30);

/// Overlay0 — "No changes" placeholder text.
pub const PLACEHOLDER: Color = Color::Rgb(0x6c, 0x70, 0x86);

/// Sky — hunk header text.
pub const HUNK: Color = Color::Rgb(0x89, 0xdc, 0xeb);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_defined() {
        assert_ne!(ADD_BG, DEL_BG);
        assert_ne!(ACCENT, ACCENT_DIM);
    }
}
