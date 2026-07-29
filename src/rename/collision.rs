use std::collections::{BTreeSet, HashMap};

use oxc_semantic::{AstNodes, ScopeId, Scoping, SymbolId};
use oxc_str::Ident;

/// Split a trailing run of ASCII decimal digits off an identifier.
///
/// `"index3"` -> `("index", Some(3))`, `"iterator"` -> `("iterator", None)`.
/// An all-digit string (impossible for a valid identifier) yields `None`.
fn split_numeric_suffix(name: &str) -> (&str, Option<u64>) {
    let bytes = name.as_bytes();
    let mut i = bytes.len();
    while i > 0 && bytes[i - 1].is_ascii_digit() {
        i -= 1;
    }
    if i == 0 || i == bytes.len() {
        (name, None)
    } else {
        (&name[..i], name[i..].parse::<u64>().ok())
    }
}

/// Resolve `base` to a free name given an `is_taken` predicate, appending or
/// **incrementing** a numeric suffix.
///
/// If `base` already ends in a number, that number is incremented rather than a
/// new one appended — so `userAccountManager2` becomes `userAccountManager3`
/// (not `userAccountManager22`), and `index3` becomes `index4`. Names without
/// a trailing number start at `2` (`index` -> `index2` -> `index3`).
///
/// Returns the resolved name and whether any suffixing was needed.
pub(crate) fn dedupe<F: Fn(&str) -> bool>(base: &str, is_taken: F) -> (String, bool) {
    if !is_taken(base) {
        return (base.to_string(), false);
    }
    let (stem, start) = split_numeric_suffix(base);
    let mut n = start.map(|x| x.saturating_add(1)).unwrap_or(2);
    loop {
        let candidate = format!("{stem}{n}");
        if !is_taken(&candidate) {
            return (candidate, true);
        }
        n += 1;
    }
}

/// Scope-aware collision resolver.
///
/// Replaces the old file-global `taken` set (which forced whole-file name
/// uniqueness and produced the `________index` underscore pileup). A candidate
/// name only collides when it is actually visible in the same scope chain:
///
///   1. **Same scope or an ancestor** — detected via `Scoping::find_binding`,
///      which walks up the scope tree. This prevents same-scope redeclaration
///      and prevents shadowing an outer binding the new name would capture.
///   2. **A descendant scope** — detected via an Euler-tour interval over the
///      scope tree. Renaming an outer symbol to a name already bound in a nested
///      scope would capture the outer symbol's references that occur inside that
///      nested scope (e.g. renaming outer `a` to `x` in
///      `var a; function f() { var x; return a; }` would silently rebind the
///      `return a` to the inner `x`), so it must count as a collision. The
///      descendant index is seeded from every existing binding and kept live by
///      `commit`, so this holds no matter what order symbols are processed in.
///   3. **An unresolved reference** — these are runtime globals or names supplied
///      by the host application. Creating a binding with the same name could
///      silently capture the existing reference.
///
/// Two identifiers in *disjoint* scopes (e.g. sibling functions) may freely
/// share a name — exactly as JavaScript allows — so no underscores accrue.
pub(crate) struct CollisionResolver {
    /// Euler-tour enter index per scope (order of first visit in a DFS).
    enter: HashMap<ScopeId, u32>,
    /// Euler-tour exit index per scope. Scope `d` is a descendant of scope `a`
    /// iff `enter[a] < enter[d] < exit[a]`.
    exit: HashMap<ScopeId, u32>,
    /// For each currently-bound name, the sorted set of enter-indices of the
    /// scopes that bind it. Enables an O(log n) "is this name bound in any
    /// descendant of scope X?" range query.
    name_scopes: HashMap<String, BTreeSet<u32>>,
    /// For each unresolved name, the scopes containing its references. These
    /// may be runtime globals or host-provided names that no static browser or
    /// Node global list could know about.
    unresolved_name_scopes: HashMap<String, BTreeSet<u32>>,
}

impl CollisionResolver {
    /// Build the scope tree Euler tour and seed the name index from every
    /// existing binding (including names that will never be renamed, so that
    /// descendant kept-names are still respected).
    pub(crate) fn new(scoping: &Scoping, nodes: &AstNodes<'_>) -> Self {
        // Build children adjacency from parent links.
        let mut children: HashMap<ScopeId, Vec<ScopeId>> = HashMap::new();
        for scope_id in scoping.scope_descendants_from_root() {
            if let Some(parent) = scoping.scope_parent_id(scope_id) {
                children.entry(parent).or_default().push(scope_id);
            }
        }

        // Iterative DFS from the root, assigning enter/exit indices so that a
        // scope's descendants occupy a contiguous open interval of enter-indices.
        let mut enter: HashMap<ScopeId, u32> = HashMap::new();
        let mut exit: HashMap<ScopeId, u32> = HashMap::new();
        let mut counter: u32 = 0;
        let root = scoping.root_scope_id();
        // Stack holds (scope, entered?). On first pop we record enter and push
        // the scope back as "entered" plus its children; on second pop we record
        // exit.
        let mut stack: Vec<(ScopeId, bool)> = vec![(root, false)];
        while let Some((scope_id, entered)) = stack.pop() {
            if entered {
                exit.insert(scope_id, counter);
                continue;
            }
            enter.insert(scope_id, counter);
            counter += 1;
            stack.push((scope_id, true));
            if let Some(kids) = children.get(&scope_id) {
                for &kid in kids {
                    stack.push((kid, false));
                }
            }
        }

        // Seed the name index from all current bindings.
        let mut name_scopes: HashMap<String, BTreeSet<u32>> = HashMap::new();
        for (scope_id, bindings) in scoping.iter_bindings() {
            let Some(&idx) = enter.get(&scope_id) else {
                continue;
            };
            for name in bindings.keys() {
                name_scopes.entry(name.to_string()).or_default().insert(idx);
            }
        }
        let mut unresolved_name_scopes = HashMap::new();
        for (name, references) in scoping.root_unresolved_references() {
            let indices = references
                .iter()
                .filter_map(|reference_id| {
                    let reference = scoping.get_reference(*reference_id);
                    let scope_id = nodes.get_node(reference.node_id()).scope_id();
                    enter.get(&scope_id).copied()
                })
                .collect();
            unresolved_name_scopes.insert(name.to_string(), indices);
        }

        CollisionResolver {
            enter,
            exit,
            name_scopes,
            unresolved_name_scopes,
        }
    }

    /// Returns true if `candidate` cannot be assigned to `symbol` (declared in
    /// `decl_scope`) without clashing with a binding visible in the same scope
    /// chain (ancestor, same scope, or descendant).
    pub(crate) fn collides(
        &self,
        scoping: &Scoping,
        decl_scope: ScopeId,
        symbol: SymbolId,
        candidate: &str,
    ) -> bool {
        // An unresolved reference is captured only when it occurs in the
        // declaration scope or one of its descendants. References in sibling
        // or ancestor scopes cannot see this new binding.
        if let (Some(&lo), Some(&hi)) = (self.enter.get(&decl_scope), self.exit.get(&decl_scope)) {
            if let Some(indices) = self.unresolved_name_scopes.get(candidate) {
                if indices.range(lo..hi).next().is_some() {
                    return true;
                }
            }
        }

        // (1) Same scope or ancestor: find_binding walks up from decl_scope.
        // A hit on the symbol itself is not a real collision (candidate == its
        // own current name, i.e. a no-op rename).
        if let Some(found) = scoping.find_binding(decl_scope, Ident::from(candidate)) {
            if found != symbol {
                return true;
            }
        }

        // (2) Descendant scopes: is `candidate` bound anywhere strictly inside
        // decl_scope's subtree? Query the open enter-index interval.
        if let (Some(&lo), Some(&hi)) = (self.enter.get(&decl_scope), self.exit.get(&decl_scope)) {
            if let Some(indices) = self.name_scopes.get(candidate) {
                // Descendants have enter-index in (lo, hi).
                if indices.range((lo + 1)..hi).next().is_some() {
                    return true;
                }
            }
        }

        false
    }

    /// Record that `decl_scope`'s binding changed from `old_name` to `new_name`,
    /// keeping the descendant index in sync for subsequent `collides` queries.
    pub(crate) fn commit(&mut self, decl_scope: ScopeId, old_name: &str, new_name: &str) {
        let Some(&idx) = self.enter.get(&decl_scope) else {
            return;
        };
        if let Some(set) = self.name_scopes.get_mut(old_name) {
            set.remove(&idx);
            if set.is_empty() {
                self.name_scopes.remove(old_name);
            }
        }
        self.name_scopes
            .entry(new_name.to_string())
            .or_default()
            .insert(idx);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{dedupe, split_numeric_suffix};

    #[test]
    fn split_no_trailing_digits() {
        assert_eq!(split_numeric_suffix("iterator"), ("iterator", None));
    }

    #[test]
    fn split_trailing_digits() {
        assert_eq!(split_numeric_suffix("index3"), ("index", Some(3)));
        assert_eq!(
            split_numeric_suffix("userAccountManager2"),
            ("userAccountManager", Some(2))
        );
    }

    #[test]
    fn split_all_digits_is_treated_as_no_suffix() {
        assert_eq!(split_numeric_suffix("123"), ("123", None));
    }

    #[test]
    fn dedupe_free_name_unchanged() {
        let taken: HashSet<&str> = HashSet::new();
        assert_eq!(
            dedupe("index", |c| taken.contains(c)),
            ("index".to_string(), false)
        );
    }

    #[test]
    fn dedupe_appends_when_no_trailing_number() {
        let taken: HashSet<String> = ["index".to_string()].into_iter().collect();
        assert_eq!(
            dedupe("index", |c| taken.contains(c)),
            ("index2".to_string(), true)
        );
    }

    #[test]
    fn dedupe_increments_existing_trailing_number() {
        // The bug the user reported: `...Manager2` colliding must become
        // `...Manager3`, not `...Manager22`.
        let taken: HashSet<String> = ["userAccountManager2".to_string()].into_iter().collect();
        assert_eq!(
            dedupe("userAccountManager2", |c| taken.contains(c)),
            ("userAccountManager3".to_string(), true)
        );
    }

    #[test]
    fn dedupe_increments_until_free() {
        let taken: HashSet<String> = ["index3".to_string(), "index4".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            dedupe("index3", |c| taken.contains(c)),
            ("index5".to_string(), true)
        );
    }
}
