use std::collections::HashMap;
use std::hash::Hash;

use gpui::{ColorExt, Hsla};

pub use debugger_model::Highlight;

use crate::theme::Theme;

pub fn highlight_color(theme: &Theme, highlight: Highlight) -> Hsla {
    match highlight {
        Highlight::None => theme.text_primary,
        Highlight::Execution => theme.accent,
        Highlight::Pipeline => theme.info,
        Highlight::Changed => theme.danger,
        Highlight::Read => theme.info,
        Highlight::Written => theme.danger,
    }
}

pub fn highlight_fill(theme: &Theme, highlight: Highlight) -> Option<(Hsla, Hsla)> {
    match highlight {
        Highlight::None => None,
        Highlight::Execution => Some((theme.accent_soft, theme.accent)),
        Highlight::Pipeline => Some((theme.info.opacity(0.16), theme.info)),
        Highlight::Read => Some((theme.read_fill, theme.read_border)),
        Highlight::Written | Highlight::Changed => Some((theme.write_fill, theme.write_border)),
    }
}

#[derive(Default)]
pub struct ChangeTracker<K: Eq + Hash> {
    previous: HashMap<K, u64>,
}

impl<K: Eq + Hash> ChangeTracker<K> {
    pub fn clear(&mut self) {
        self.previous.clear();
    }

    pub fn highlight(&mut self, key: K, value: u64) -> Highlight {
        match self.previous.insert(key, value) {
            Some(prev) if prev != value => Highlight::Changed,
            _ => Highlight::None,
        }
    }
}
