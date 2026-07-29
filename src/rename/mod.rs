mod collision;
mod safe_name;
#[cfg(test)]
pub mod test_dsl;
mod walker;

pub use walker::{rename_all_identifiers, rename_all_identifiers_with_observer};

pub trait Renamer {
    /// Returns the new name for the identifier. Returning the same string means "leave it alone".
    fn rename(&mut self, original: &str, surrounding_code: &str) -> String;
}

pub trait RenameObserver {
    fn identifiers_found(&mut self, _total: usize) {}

    fn rename_started(&mut self, _current: usize, _total: usize, _original: &str) {}

    fn rename_finished(&mut self, _current: usize, _total: usize, _original: &str, _renamed: &str) {
    }
}

pub struct NoopRenameObserver;

impl RenameObserver for NoopRenameObserver {}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("failed to parse JavaScript: {0}")]
    Parse(String),
}
