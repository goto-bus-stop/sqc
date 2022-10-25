//! Format SQL strings with highlighting.

use std::cell::RefCell;
use std::io::Write;
use termcolor::{Buffer, Color, ColorSpec, WriteColor};
use tree_sitter_highlight::{
    Error as HighlightError, Highlight, HighlightConfiguration, HighlightEvent, Highlighter,
};

const QUERY_NAMES: [&str; 8] = [
    "keyword",
    "number",
    "string",
    "constant",
    "comment",
    "operator",
    "punctuation",
    "variable",
];

pub struct SqlHighlighter {
    highlighter: RefCell<Highlighter>,
    sql_config: HighlightConfiguration,
}

impl SqlHighlighter {
    pub fn new() -> Self {
        let highlighter = Highlighter::new();
        let mut sql_config = HighlightConfiguration::new(
            tree_sitter_sqlite::language(),
            tree_sitter_sqlite::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .unwrap();
        sql_config.configure(&QUERY_NAMES);

        Self {
            highlighter: RefCell::new(highlighter),
            sql_config,
        }
    }

    pub fn highlight(&self, sql: &str) -> anyhow::Result<String> {
        let mut highlighter = self.highlighter.borrow_mut();
        let highlights = highlighter.highlight(&self.sql_config, sql.as_bytes(), None, |_| None)?;
        to_ansi(sql.as_bytes(), highlights)
    }
}

impl Default for SqlHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

/// Turn highlights into ANSI sequences. Accepts highlights in any language, but the name order
/// needs to match.
pub fn to_ansi(
    source: &[u8],
    highlights: impl Iterator<Item = Result<HighlightEvent, HighlightError>>,
) -> anyhow::Result<String> {
    let mut buf = Buffer::ansi();

    let mut keyword = ColorSpec::new();
    keyword.set_fg(Some(Color::Blue)).set_bold(true);
    let mut number = ColorSpec::new();
    number.set_fg(Some(Color::Yellow)).set_bold(true);
    let mut string = ColorSpec::new();
    string.set_fg(Some(Color::Magenta)).set_bold(true);
    let mut comment = ColorSpec::new();
    comment.set_fg(Some(Color::Green)).set_bold(true);
    let mut parameter = ColorSpec::new();
    parameter.set_fg(Some(Color::Magenta)).set_bold(true);

    for event in highlights {
        match event? {
            HighlightEvent::HighlightStart(Highlight(style)) => match style {
                0 => buf.set_color(&keyword)?,
                1 => buf.set_color(&number)?,
                2 => buf.set_color(&string)?,
                4 => buf.set_color(&comment)?,
                7 => buf.set_color(&parameter)?,
                _ => (),
            },
            HighlightEvent::Source { start, end } => buf.write_all(&source[start..end])?,
            HighlightEvent::HighlightEnd => buf.reset()?,
        }
    }

    Ok(String::from_utf8(buf.into_inner())?)
}
