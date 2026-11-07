//! Minimal ANSI colour handling.

use std::io::IsTerminal;

#[derive(Debug, Clone, Copy)]
pub struct Color {
    enabled: bool,
}

impl Color {
    pub fn new(disabled: bool) -> Self {
        let enabled =
            !disabled && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal();
        Self { enabled }
    }

    pub fn paint(&self, code: &str, text: impl AsRef<str>) -> String {
        if self.enabled {
            format!("\x1b[{code}m{}\x1b[0m", text.as_ref())
        } else {
            text.as_ref().to_string()
        }
    }

    pub fn green(&self, text: impl AsRef<str>) -> String {
        self.paint("32", text)
    }

    pub fn yellow(&self, text: impl AsRef<str>) -> String {
        self.paint("33", text)
    }

    pub fn red(&self, text: impl AsRef<str>) -> String {
        self.paint("31", text)
    }

    pub fn blue(&self, text: impl AsRef<str>) -> String {
        self.paint("34", text)
    }

    pub fn dim(&self, text: impl AsRef<str>) -> String {
        self.paint("2", text)
    }
}
