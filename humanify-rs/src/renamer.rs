//! Renamer trait and related types for determining new variable names

/// Context provided to the renamer for making naming decisions
#[derive(Debug, Clone)]
pub struct RenameContext<'a> {
    /// The original identifier name (e.g., "a", "fn1")
    pub name: &'a str,
    /// The surrounding code context where the identifier is defined
    pub surrounding_code: &'a str,
}

/// Trait for determining new names for identifiers
///
/// Implement this trait to provide custom renaming logic.
/// The renamer receives context about each identifier and returns
/// a new name (or the same name to keep it unchanged).
pub trait Renamer {
    /// Determine a new name for an identifier
    ///
    /// # Arguments
    /// * `ctx` - Context containing the identifier name and surrounding code
    ///
    /// # Returns
    /// The new name for the identifier, or the same name to keep it unchanged
    fn rename(&mut self, ctx: RenameContext<'_>) -> String;
}

/// A simple renamer that applies a function to determine new names
pub struct FnRenamer<F>
where
    F: FnMut(&str, &str) -> String,
{
    rename_fn: F,
}

impl<F> FnRenamer<F>
where
    F: FnMut(&str, &str) -> String,
{
    /// Create a new function-based renamer
    pub fn new(rename_fn: F) -> Self {
        Self { rename_fn }
    }
}

impl<F> Renamer for FnRenamer<F>
where
    F: FnMut(&str, &str) -> String,
{
    fn rename(&mut self, ctx: RenameContext<'_>) -> String {
        (self.rename_fn)(ctx.name, ctx.surrounding_code)
    }
}

/// A no-op renamer that returns the same name
pub struct NoOpRenamer;

impl Renamer for NoOpRenamer {
    fn rename(&mut self, ctx: RenameContext<'_>) -> String {
        ctx.name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fn_renamer_works() {
        let mut renamer = FnRenamer::new(|name, _ctx| format!("{}_renamed", name));
        let result = renamer.rename(RenameContext {
            name: "foo",
            surrounding_code: "const foo = 1;",
        });
        assert_eq!(result, "foo_renamed");
    }

    #[test]
    fn noop_renamer_works() {
        let mut renamer = NoOpRenamer;
        let result = renamer.rename(RenameContext {
            name: "bar",
            surrounding_code: "const bar = 2;",
        });
        assert_eq!(result, "bar");
    }
}
