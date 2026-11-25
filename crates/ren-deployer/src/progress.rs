//! What xtask tells the operator while it works.

/// One line of progress on the terminal: `==> <message>`.
#[must_use]
pub struct Progress<'a>(pub &'a str);

impl Progress<'_> {
    pub fn show(self) {
        println!("==> {}", self.0);
    }
}
