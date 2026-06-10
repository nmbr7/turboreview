use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

struct Assets {
    syntaxes: SyntaxSet,
    theme: Theme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let mut themes = ThemeSet::load_defaults();
        // base16-ocean.dark is bundled in syntect's default ThemeSet.
        let theme = themes
            .themes
            .remove("base16-ocean.dark")
            .expect("bundled theme present");
        Assets { syntaxes, theme }
    })
}

/// Convert a syntect RGBA colour to a ratatui Color.
/// Alpha == 0 means "no colour" in syntect conventions.
fn syntect_color_to_ratatui(c: syntect::highlighting::Color) -> Option<Color> {
    // syntect uses alpha == 0 as the "unset / no color" sentinel, not transparency.
    if c.a > 0 {
        Some(Color::Rgb(c.r, c.g, c.b))
    } else {
        None
    }
}

/// Convert syntect FontStyle to ratatui Modifier.
fn syntect_font_style_to_modifier(fs: FontStyle) -> Modifier {
    let mut m = Modifier::empty();
    if fs.contains(FontStyle::BOLD) {
        m |= Modifier::BOLD;
    }
    if fs.contains(FontStyle::ITALIC) {
        m |= Modifier::ITALIC;
    }
    if fs.contains(FontStyle::UNDERLINE) {
        m |= Modifier::UNDERLINED;
    }
    m
}

/// Convert a syntect (Style, &str) segment into an owned ratatui Span<'static>.
fn segment_to_span(style: syntect::highlighting::Style, content: &str) -> Span<'static> {
    let fg = syntect_color_to_ratatui(style.foreground);
    let bg = syntect_color_to_ratatui(style.background);
    let modifier = syntect_font_style_to_modifier(style.font_style);
    let mut ratatui_style = Style::default().add_modifier(modifier);
    if let Some(fg) = fg {
        ratatui_style = ratatui_style.fg(fg);
    }
    if let Some(bg) = bg {
        ratatui_style = ratatui_style.bg(bg);
    }
    Span::styled(content.to_owned(), ratatui_style)
}

/// Highlight one line of code, choosing the grammar by file extension.
/// Falls back to a single plain span if the extension is unknown or highlighting fails.
pub fn highlight_code(text: &str, extension: &str) -> Vec<Span<'static>> {
    let assets = assets();
    let syntax = assets
        .syntaxes
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| assets.syntaxes.find_syntax_plain_text());
    let mut hl = HighlightLines::new(syntax, &assets.theme);
    match hl.highlight_line(text, &assets.syntaxes) {
        Ok(ranges) => ranges
            .into_iter()
            .map(|(style, content)| segment_to_span(style, content))
            .collect(),
        Err(_) => vec![Span::raw(text.to_owned())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_extension_produces_spans() {
        let spans = highlight_code("let x = 42;", "rs");
        assert!(!spans.is_empty());
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("let"));
    }

    #[test]
    fn unknown_extension_falls_back_without_panic() {
        let spans = highlight_code("anything goes", "zzz-unknown");
        let joined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(joined.contains("anything goes"));
    }
}
