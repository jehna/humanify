mod safe_name;
mod walker;

pub use walker::rename_all_identifiers;

pub trait Renamer {
    /// Returns the new name for the identifier. Returning the same string means "leave it alone".
    fn rename(&mut self, original: &str, surrounding_code: &str) -> String;
}

#[derive(Debug, thiserror::Error)]
pub enum RenameError {
    #[error("failed to parse JavaScript: {0}")]
    Parse(String),
}
