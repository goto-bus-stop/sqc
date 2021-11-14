use crate::completions::Completions;
use crate::format::highlight_sql;
use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::{Context, Helper};
use std::borrow::Cow;

pub struct EditorHelper {
    name: Option<String>,
    completions: Completions,
}

impl EditorHelper {
    pub fn new(name: Option<String>, completions: Completions) -> Self {
        Self { name, completions }
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Highlighter for EditorHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        match highlight_sql(line) {
            Ok(highlighted) => highlighted.into(),
            Err(_) => line.into(),
        }
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        hint.into()
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
