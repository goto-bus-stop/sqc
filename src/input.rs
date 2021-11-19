use crate::completions::Completions;
use crate::highlight::SqlHighlighter;
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};
use std::borrow::Cow;

pub struct EditorHelper {
    name: Option<String>,
    completions: Completions,
    pub highlighter: SqlHighlighter,
}

impl EditorHelper {
    pub fn new(name: Option<String>, completions: Completions) -> Self {
        Self {
            name,
            completions,
            highlighter: Default::default(),
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Highlighter for EditorHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        match self.highlighter.highlight(line) {
            Ok(highlighted) => highlighted.into(),
            Err(_) => line.into(),
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        use std::io::Write;
        use termcolor::{Buffer, Color, ColorSpec, WriteColor};

        let mut grey = ColorSpec::new();
        grey.set_fg(Some(Color::Ansi256(8))).set_bold(true);
        let mut buf = Buffer::ansi();
        let _ = buf.set_color(&grey);
        let _ = buf.write_all(hint.as_bytes());
        let _ = buf.reset();

        String::from_utf8(buf.into_inner())
            .map(Cow::Owned)
            .unwrap_or(Cow::Borrowed(hint))
    }

    fn highlight_char(&self, line: &str, _pos: usize) -> bool {
        !line.starts_with('.')
    }
}

impl Completer for EditorHelper {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Self::Candidate>)> {
        let results = self.completions.get_completions(line, pos);
        if let Some(first) = results.get(0) {
            Ok((first.0, results.into_iter().map(|item| item.1).collect()))
        } else {
            Ok((0, vec![]))
        }
    }
}

impl Hinter for EditorHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        self.completions
            .get_completions(line, pos)
            .into_iter()
            .next()
            .map(|mut item| item.1.split_off(pos - item.0))
    }
}

impl Validator for EditorHelper {}

impl Helper for EditorHelper {}
