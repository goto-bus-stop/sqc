use rustyline::completion::Completer;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::validate::Validator;
use rustyline::Helper;
use std::borrow::Cow;

use crate::format::highlight_sql;

pub struct EditorHelper {
    pub name: Option<String>,
}

impl EditorHelper {
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
    fn highlight_char(&self, line: &str, _pos: usize) -> bool {
        !line.starts_with('.')
    }
}
impl Completer for EditorHelper {
    type Candidate = String;
}
impl Hinter for EditorHelper {
    type Hint = String;
}
impl Validator for EditorHelper {}

impl Helper for EditorHelper {}
