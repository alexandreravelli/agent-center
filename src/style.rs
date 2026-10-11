//! Terminal colors: on when the output is a terminal, `NO_COLOR` is not set
//! (<https://no-color.org>) and `TERM` is not `dumb`.

use std::io::IsTerminal;

/// Paints text for one output stream, or leaves it as it is.
#[derive(Clone, Copy, Debug)]
pub struct Style {
    on: bool,
}

impl Style {
    /// Colors for standard output.
    pub fn stdout() -> Style {
        Style::new(std::io::stdout().is_terminal())
    }

    /// Colors for standard error.
    pub fn stderr() -> Style {
        Style::new(std::io::stderr().is_terminal())
    }

    fn new(terminal: bool) -> Style {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let dumb = std::env::var_os("TERM").is_some_and(|t| t == "dumb");
        Style { on: terminal && !no_color && !dumb }
    }

    fn paint(self, sgr: &str, text: &str) -> String {
        if self.on { format!("\x1b[{sgr}m{text}\x1b[0m") } else { text.to_string() }
    }

    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn red(self, text: &str) -> String {
        self.paint("1;31", text)
    }

    pub fn green(self, text: &str) -> String {
        self.paint("32", text)
    }

    pub fn yellow(self, text: &str) -> String {
        self.paint("33", text)
    }

    /// The "==> step" lines.
    pub fn step(self, text: &str) -> String {
        self.paint("1;36", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_only_when_on() {
        assert_eq!(Style { on: false }.green("1.0"), "1.0");
        assert_eq!(Style { on: true }.green("1.0"), "\x1b[32m1.0\x1b[0m");
        assert_eq!(Style { on: true }.red("x"), "\x1b[1;31mx\x1b[0m");
    }
}
