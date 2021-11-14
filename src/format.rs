//! Format SQL strings with highlighting.

use tree_sitter::{Parser, Tree};
use tree_sitter_highlight::{Highlight, Highlighter, HighlightConfiguration, HighlightEvent};
use termcolor::{Buffer, Color, ColorSpec, WriteColor};
use std::io::Write;

pub fn parse_sql(sql: &str) -> anyhow::Result<Tree> {
    let mut parser = Parser::new();
    parser.set_language(tree_sitter_sqlite::language())?;
    let tree = parser.parse(sql, None).unwrap();
    Ok(tree)
}

pub fn highlight_sql(sql: &str) -> anyhow::Result<String> {
    let mut highlighter = Highlighter::new();
    let mut sql_config = HighlightConfiguration::new(
        tree_sitter_sqlite::language(),
        include_str!("../../tree-sitter-sqlite/queries/highlights.scm"),
        "",
        "",
    ).unwrap();
    sql_config.configure(&[
                         "keyword", "number", "string", "constant", "comment", "operator", "punctuation",
    ]);

    let highlights = highlighter.highlight(&sql_config, &sql.as_bytes(), None, |_| None)?;
    let mut buf = Buffer::ansi();

    let mut keyword = ColorSpec::new();
    keyword.set_fg(Some(Color::Blue)).set_bold(true);
    let mut number = ColorSpec::new();
    number.set_fg(Some(Color::Yellow)).set_bold(true);
    let mut string = ColorSpec::new();
    string.set_fg(Some(Color::Magenta)).set_bold(true);
    let mut comment = ColorSpec::new();
    comment.set_fg(Some(Color::Green)).set_bold(true);

    for event in highlights {
        match event? {
            HighlightEvent::HighlightStart(Highlight(style)) => {
                match style {
                    0 => buf.set_color(&keyword)?,
                    1 => buf.set_color(&number)?,
                    2 => buf.set_color(&string)?,
                    4 => buf.set_color(&comment)?,
                    _ => (),
                }
            },
            HighlightEvent::Source { start, end } => buf.write_all(&sql.as_bytes()[start..end])?,
            HighlightEvent::HighlightEnd => buf.reset()?,
        }
    }

    Ok(String::from_utf8(buf.into_inner())?)
}
