use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::rename::{RenameOutcome, RenameRequest, Renamer, BUDGET_EXHAUSTED_REASON};

pub const CACHE_FORMAT: &str = "humanify-cache-v1";

/// Every key is the hex sha256 produced by [`cache_key`]. `get`/`put` are
/// public and both slice the key (`&key[..2]` for the shard) and interpolate it
/// into a path, so anything that is not exactly 64 lowercase hex characters is
/// rejected up front: a non-ASCII key would panic on a non-char-boundary slice,
/// and one containing `..` or a separator would escape the cache root.
fn is_valid_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Length-prefixed sha256 so that no two different field splits can collide
/// (`("ab","c")` and `("a","bc")` must hash differently).
pub fn sha256_hex_parts(parts: &[&[u8]]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    let mut out = String::with_capacity(64);
    for b in h.finalize() {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Provenance recorded on every entry: *who* was asked. Deliberately **not**
/// part of the cache key — see [`cache_key`].
#[derive(Clone, Debug)]
pub struct CacheScope {
    pub provider: &'static str,
    pub model: String,
    pub base_url: String,
    pub json_mode: String,
}

/// Canonicalises a context slice so that code differing only in layout hashes
/// identically. Applied to the cache key and to `cache_context_sha256`, never to
/// what is sent to the model.
///
/// Three rules:
/// 1. Any run of whitespace and/or `;` collapses to a single space.
/// 2. That space is then dropped unless it sits between two identifier
///    characters (`[A-Za-z0-9_$]`). This is what turns `a = 1` into `a=1` and
///    `f(a, b)` into `f(a,b)` while leaving `return x`, `typeof x`, `new Foo`
///    and `case 1:` intact — collapsing those would fuse two tokens into one
///    and make the slice hash like unrelated code.
/// 3. `'` is folded to `"`, so a formatter's quote-style preference does not
///    change the key. The quotes themselves are *kept*: dropping them would make
///    `const a = "userId"` and `const a = userId` collide, and those deserve
///    different names.
///
/// The result is deliberately not valid JavaScript — `a;b` and `a b` both become
/// `a b`, which differ under ASI. That is acceptable because the cached artefact
/// is a *name suggestion*: a wrong match yields a misleading identifier, never
/// broken output, since every name still goes through `safe_name` and
/// `CollisionResolver`.
///
/// Note the limit on how far this reaches. `compute_context_window` truncates in
/// raw bytes *before* this runs, and formatting inflates byte counts, so the
/// truncating branches see different amounts of code in a formatted file than in
/// a minified one — and a scope near the `--context-size` boundary can even
/// switch branches. Reformatting-resilience is therefore reliable for scopes that
/// fit inside the context window and best-effort beyond it. Making it complete
/// would mean normalising the scope text *before* truncating it, which the walker
/// would have to do since only it holds the scope span.
pub fn normalize_for_key(s: &str) -> String {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '$';
    let is_collapsible = |c: char| c.is_whitespace() || c == ';';

    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut last: Option<char> = None;

    while let Some(c) = chars.next() {
        if is_collapsible(c) {
            while chars.peek().is_some_and(|&n| is_collapsible(n)) {
                chars.next();
            }
            // Keep a separator only where removing it would weld two identifiers
            // together. A run at either end of the slice has no neighbour on one
            // side and is dropped.
            if let (Some(prev), Some(&next)) = (last, chars.peek()) {
                if is_ident(prev) && is_ident(next) {
                    out.push(' ');
                    last = Some(' ');
                }
            }
        } else {
            let c = if c == '\'' { '"' } else { c };
            out.push(c);
            last = Some(c);
        }
    }
    out
}

/// The key identifies the *code being asked about* and nothing else: the
/// identifier and its context slice, the latter normalised by
/// [`normalize_for_key`]. Everything describing **how** it was asked — provider,
/// model, base URL, JSON mode, prompt and schema — is intentionally excluded, so
/// any answer for a given identifier-in-context is reused by any later run.
///
/// That is what makes a mixed-cost workflow possible: run an expensive model over
/// one region of a bundle, then a cheap model over the whole file, and the
/// expensive answers survive rather than being re-bought.
///
/// The two escape hatches replace what key partitioning used to do implicitly:
/// `--refresh-cache` re-asks and overwrites, and `--cache-dir` gives a run its own
/// namespace. Note the consequence for upgrades — a humanify release that changes
/// the prompt no longer invalidates anything by itself, so a warm cache keeps
/// serving answers produced by the older prompt until it is refreshed.
///
/// Everything omitted here is still *recorded* on each entry (model, provider,
/// base URL, JSON mode, prompt fingerprint), so the origin of any cached name
/// stays inspectable even though it no longer partitions the namespace.
pub fn cache_key(original: &str, surrounding: &str) -> String {
    sha256_hex_parts(&[
        CACHE_FORMAT.as_bytes(),
        original.as_bytes(),
        normalize_for_key(surrounding).as_bytes(),
    ])
}

/// The re-verification hash stored on an entry. Two rules, both load-bearing
/// because `DiskCache::get` *deletes* an entry whose stored hash does not
/// verify — a mismatch is not a quiet miss, it destroys the entry:
///
/// 1. It **must** normalise the same way [`cache_key`] does, or two
///    layout-variant contexts would thrash, each lookup destroying the other's
///    entry.
/// 2. It **must** be computed from the same window as the key — the *cache*
///    context, not the prompt context. Under `--cache-context-size` those two
///    differ, so hashing the prompt window here would give two runs at different
///    `--context-size` values the same key but different hashes: every lookup a
///    miss *and* a deletion of the other run's work. That is strictly worse than
///    not having the flag, which is why this takes the key window by name.
pub fn cache_context_fingerprint(cache_context: &str) -> String {
    sha256_hex_parts(&[normalize_for_key(cache_context).as_bytes()])
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub key: String,
    pub name: String,
    pub original: String,
    pub cache_context_sha256: String,
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub json_mode: String,
    pub prompt_fingerprint: String,
    pub created_at_unix: u64,
    /// The prompt window this answer was produced with, and the key window it is
    /// filed under. Diagnostics only — neither is part of the key, and an entry
    /// written at one pair is deliberately served to a run using another. They
    /// are what makes "why did these two runs not share entries?" answerable
    /// after the fact.
    ///
    /// `None` on entries written before these fields existed; `serde(default)`
    /// so such an entry still parses rather than being read as corrupt and
    /// deleted, which would silently re-buy every name in it.
    #[serde(default)]
    pub context_size: Option<usize>,
    #[serde(default)]
    pub cache_context_size: Option<usize>,
}

impl CacheEntry {
    pub fn new(
        key: &str,
        name: &str,
        original: &str,
        cache_context_sha256: &str,
        scope: &CacheScope,
    ) -> Self {
        let created_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            key: key.to_string(),
            name: name.to_string(),
            original: original.to_string(),
            cache_context_sha256: cache_context_sha256.to_string(),
            provider: scope.provider.to_string(),
            model: scope.model.clone(),
            base_url: scope.base_url.clone(),
            json_mode: scope.json_mode.clone(),
            prompt_fingerprint: crate::llm::renamer::prompt_fingerprint().to_string(),
            created_at_unix,
            context_size: None,
            cache_context_size: None,
        }
    }

    /// Records the two window sizes this entry was produced under. Kept off
    /// [`CacheEntry::new`] so it stays clear that they are provenance, not part
    /// of the identity checked by [`CacheEntry::matches`].
    pub fn with_context_sizes(mut self, context_size: usize, cache_context_size: usize) -> Self {
        self.context_size = Some(context_size);
        self.cache_context_size = Some(cache_context_size);
        self
    }

    /// Re-verification on read: an entry is only usable if it records the same
    /// question that is being asked. This is what makes a sha256 collision (or a
    /// hand-edited file) degrade to "call the model again" rather than to
    /// "return the wrong name".
    ///
    /// Note what is *not* checked: the recorded context sizes. Serving an entry
    /// written at another size is the entire point of `--cache-context-size`.
    fn matches(&self, key: &str, original: &str, cache_context_sha: &str) -> bool {
        self.key == key
            && self.original == original
            && self.cache_context_sha256 == cache_context_sha
    }
}

#[derive(Clone, Debug)]
pub struct DiskCache {
    root: PathBuf,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(1);

impl DiskCache {
    /// Creates `<dir>/humanify-cache-v1`. Returns Err only if that fails.
    pub fn open(dir: &Path) -> io::Result<Self> {
        let root = dir.join(CACHE_FORMAT);
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn entry_path(&self, key: &str) -> PathBuf {
        // Sharded by the first 2 hex chars: an 11 MB bundle can produce >10 000
        // entries, and NTFS enumeration degrades badly in one flat directory.
        self.root.join(&key[..2]).join(format!("{key}.json"))
    }

    pub fn get(&self, key: &str, original: &str, cache_context_sha: &str) -> Option<String> {
        if !is_valid_key(key) {
            return None;
        }
        let file_path = self.entry_path(key);
        let bytes = fs::read(&file_path).ok()?;
        match serde_json::from_slice::<CacheEntry>(&bytes) {
            Ok(entry) if entry.matches(key, original, cache_context_sha) => Some(entry.name),
            Ok(_) => {
                // Parses, but describes a different question than the one asked —
                // a hash collision or a hand-edited file. Degrade to a miss, and
                // delete it so the answer we are about to fetch can take its
                // place. Leaving it would poison this key forever: `put`'s rename
                // cannot replace an existing file on Windows, so every future run
                // would re-pay for the same LLM call.
                let _ = fs::remove_file(&file_path);
                None
            }
            Err(_) => {
                // Corrupt / truncated JSON. Same treatment.
                let _ = fs::remove_file(&file_path);
                None
            }
        }
    }

    /// Writes `entry` atomically: a temp file in the *same* shard directory
    /// (cross-directory rename is not atomic), then a rename into place.
    pub fn put(&self, entry: &CacheEntry) -> io::Result<()> {
        if !is_valid_key(&entry.key) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache key must be 64 lowercase hex characters",
            ));
        }
        let shard_dir = self.root.join(&entry.key[..2]);
        fs::create_dir_all(&shard_dir)?;

        let final_path = self.entry_path(&entry.key);
        let pid = std::process::id();
        let cnt = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = shard_dir.join(format!("{}.{pid}.{cnt}.tmp", entry.key));

        let data =
            serde_json::to_vec(entry).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&tmp_path, &data)?;

        // From here on every exit path removes the temp file, so a failing disk
        // cannot accumulate `.tmp` debris across a long run.
        let result = self.commit(&tmp_path, &final_path, entry);
        let _ = fs::remove_file(&tmp_path);
        result
    }

    fn commit(&self, tmp_path: &Path, final_path: &Path, entry: &CacheEntry) -> io::Result<()> {
        match fs::rename(tmp_path, final_path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                // A genuine failure (permissions, disk full, a *directory* on the
                // key path) must surface as an error rather than being swallowed
                // as "someone beat us to it".
                if !final_path.is_file() {
                    return Err(e);
                }
            }
        }

        // Windows `fs::rename` refuses an existing destination (Unix would have
        // silently overwritten it, so without this the two platforms disagree
        // about who wins). Someone else got here first, or a stale entry is
        // sitting on this key. Accept the existing file only if it actually
        // answers the same question with the same answer — anything else gets
        // replaced, so a poisoned key repairs itself on the next run.
        if let Ok(bytes) = fs::read(final_path) {
            if let Ok(existing) = serde_json::from_slice::<CacheEntry>(&bytes) {
                if existing.matches(&entry.key, &entry.original, &entry.cache_context_sha256)
                    && existing.name == entry.name
                {
                    return Ok(());
                }
            }
        }

        // Not equivalent: replace it. `remove` + `rename` is not atomic, but the
        // window is tiny and a reader that catches it sees a missing file, which
        // is just a miss.
        fs::remove_file(final_path)?;
        match fs::rename(tmp_path, final_path) {
            Ok(()) => Ok(()),
            // Lost a race with another writer that re-created the destination.
            // Their entry is as good as ours; don't fight over it.
            Err(e) if final_path.exists() => {
                let _ = e;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[derive(Default, Debug)]
pub struct CacheStats {
    pub hits: AtomicUsize,
    pub misses: AtomicUsize,
    pub writes: AtomicUsize,
    pub write_errors: AtomicUsize,
    /// Lookups skipped because `--refresh-cache` is on. Counted separately from
    /// `misses`: the cache was never consulted, so calling it a miss would
    /// misreport a cache that is in fact full of usable entries.
    pub bypassed: AtomicUsize,
}

pub struct CachingRenamer {
    inner: Box<dyn Renamer + Send>,
    cache: DiskCache,
    scope: CacheScope,
    stats: Arc<CacheStats>,
    warned: bool,
    refresh: bool,
    context_sizes: Option<(usize, usize)>,
}

impl CachingRenamer {
    pub fn new(inner: Box<dyn Renamer + Send>, cache: DiskCache, scope: CacheScope) -> Self {
        Self {
            inner,
            cache,
            scope,
            stats: Arc::new(CacheStats::default()),
            warned: false,
            refresh: false,
            context_sizes: None,
        }
    }

    /// Records `(--context-size, --cache-context-size)` on every entry this
    /// renamer writes. Provenance only; it never affects a key or a lookup.
    pub fn with_context_sizes(mut self, context_size: usize, cache_context_size: usize) -> Self {
        self.context_sizes = Some((context_size, cache_context_size));
        self
    }

    /// `--refresh-cache`: ignore stored answers, but keep writing fresh ones, so
    /// a re-run re-asks the model and overwrites what is there. Distinct from
    /// `--no-cache`, which disables reads *and* writes.
    pub fn with_refresh(mut self, refresh: bool) -> Self {
        self.refresh = refresh;
        self
    }

    pub fn stats(&self) -> Arc<CacheStats> {
        Arc::clone(&self.stats)
    }

    fn warn_once(&mut self, e: &io::Error) {
        if !self.warned {
            self.warned = true;
            eprintln!("humanify: cache write failed ({e}); continuing without persistence");
        }
    }
}

impl Renamer for CachingRenamer {
    fn rename(&mut self, original: &str, surrounding: &str) -> String {
        match self.try_rename(&RenameRequest::new(original, surrounding)) {
            RenameOutcome::Ok(n) => n,
            RenameOutcome::Failed { .. } | RenameOutcome::Skipped { .. } => original.to_string(),
        }
    }

    fn try_rename(&mut self, req: &RenameRequest<'_>) -> RenameOutcome {
        let original = req.original;
        if original.is_empty() {
            return RenameOutcome::Ok(String::new());
        }
        // Both derived from `cache_context`, never from `surrounding` — see
        // `cache_context_fingerprint`. Under `--cache-context-size` the model
        // still sees `req.surrounding`; only the filing changes.
        let cache_context_sha = cache_context_fingerprint(req.cache_context);
        let key = cache_key(original, req.cache_context);

        if self.refresh {
            self.stats.bypassed.fetch_add(1, Ordering::Relaxed);
        } else {
            if let Some(name) = self.cache.get(&key, original, &cache_context_sha) {
                self.stats.hits.fetch_add(1, Ordering::Relaxed);
                return RenameOutcome::Ok(name);
            }
            self.stats.misses.fetch_add(1, Ordering::Relaxed);
        }

        let outcome = self.inner.try_rename(req);
        if let RenameOutcome::Ok(ref name) = outcome {
            let mut entry = CacheEntry::new(&key, name, original, &cache_context_sha, &self.scope);
            if let Some((ctx, cache_ctx)) = self.context_sizes {
                entry = entry.with_context_sizes(ctx, cache_ctx);
            }
            match self.cache.put(&entry) {
                Ok(()) => {
                    self.stats.writes.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => {
                    self.stats.write_errors.fetch_add(1, Ordering::Relaxed);
                    self.warn_once(&e);
                }
            }
        }
        outcome
    }
}

pub struct BudgetRenamer {
    inner: Box<dyn Renamer + Send>,
    deadline: Option<Instant>,
    skipped: Arc<AtomicUsize>,
}

impl BudgetRenamer {
    pub fn new(inner: Box<dyn Renamer + Send>, deadline: Option<Instant>) -> Self {
        Self {
            inner,
            deadline,
            skipped: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.load(Ordering::Relaxed)
    }
}

impl Renamer for BudgetRenamer {
    fn rename(&mut self, original: &str, surrounding: &str) -> String {
        match self.try_rename(&RenameRequest::new(original, surrounding)) {
            RenameOutcome::Ok(n) => n,
            RenameOutcome::Failed { .. } | RenameOutcome::Skipped { .. } => original.to_string(),
        }
    }

    fn try_rename(&mut self, req: &RenameRequest<'_>) -> RenameOutcome {
        if let Some(dl) = self.deadline {
            if Instant::now() >= dl {
                self.skipped.fetch_add(1, Ordering::Relaxed);
                return RenameOutcome::Skipped {
                    reason: BUDGET_EXHAUSTED_REASON.to_string(),
                };
            }
        }
        // Forwarded whole: rebuilding the request here would drop the key window
        // before it ever reaches `CachingRenamer`, which sits outside this one.
        self.inner.try_rename(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;
    use std::time::Duration;
    use tempfile::TempDir;

    /// A request whose prompt window and key window are the same string — the
    /// shape every call has when `--cache-context-size` is not in play.
    fn req<'a>(original: &'a str, surrounding: &'a str) -> RenameRequest<'a> {
        RenameRequest::new(original, surrounding)
    }

    /// A request that keys on a *different* window than the model is shown, which
    /// is the whole point of `--cache-context-size`.
    fn keyed_req<'a>(
        original: &'a str,
        surrounding: &'a str,
        cache_context: &'a str,
    ) -> RenameRequest<'a> {
        RenameRequest::with_cache_context(original, surrounding, cache_context)
    }

    struct CountingRenamer {
        calls: Arc<AtomicUsize>,
        outcome: RenameOutcome,
    }

    impl CountingRenamer {
        fn new(outcome: RenameOutcome) -> (Self, Arc<AtomicUsize>) {
            let calls = Arc::new(AtomicUsize::new(0));
            (
                Self {
                    calls: Arc::clone(&calls),
                    outcome,
                },
                calls,
            )
        }
    }

    impl Renamer for CountingRenamer {
        fn rename(&mut self, original: &str, _: &str) -> String {
            match &self.outcome {
                RenameOutcome::Ok(s) => s.clone(),
                _ => original.to_string(),
            }
        }
        fn try_rename(&mut self, _req: &RenameRequest<'_>) -> RenameOutcome {
            self.calls.fetch_add(1, SeqCst);
            self.outcome.clone()
        }
    }

    fn test_scope(provider: &'static str, model: &str) -> CacheScope {
        CacheScope {
            provider,
            model: model.to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            json_mode: "ladder".to_string(),
        }
    }

    #[test]
    fn miss_then_hit_same_key() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("newName".into()));
        let mut renamer = CachingRenamer::new(Box::new(inner), cache, scope);

        let res1 = renamer.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res1, RenameOutcome::Ok("newName".into()));
        assert_eq!(count.load(SeqCst), 1);
        assert_eq!(renamer.stats().hits.load(SeqCst), 0);
        assert_eq!(renamer.stats().misses.load(SeqCst), 1);

        let res2 = renamer.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res2, RenameOutcome::Ok("newName".into()));
        assert_eq!(count.load(SeqCst), 1);
        assert_eq!(renamer.stats().hits.load(SeqCst), 1);
        assert_eq!(renamer.stats().misses.load(SeqCst), 1);
    }

    #[test]
    fn different_surrounding_is_a_miss() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("newName".into()));
        let mut renamer = CachingRenamer::new(Box::new(inner), cache, scope);

        renamer.try_rename(&req("a", "const a = 1;"));
        renamer.try_rename(&req("a", "const a = 2;"));
        assert_eq!(count.load(SeqCst), 2);
    }

    /// The point of leaving the model out of the key: an expensive model's answer
    /// for one region is reused when a cheap model is later run over the whole
    /// file, instead of being re-bought.
    #[test]
    fn different_model_is_a_hit() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope1 = test_scope("openai", "expensive-model");
        let scope2 = test_scope("openai", "cheap-model");

        let (inner1, count1) = CountingRenamer::new(RenameOutcome::Ok("goodName".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope1);
        r1.try_rename(&req("a", "const a = 1;"));
        assert_eq!(count1.load(SeqCst), 1);

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("worseName".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope2);
        assert_eq!(
            r2.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("goodName".into()),
            "the second model must reuse the first model's answer"
        );
        assert_eq!(count2.load(SeqCst), 0, "no second call should be made");
    }

    #[test]
    fn different_base_url_is_a_hit() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let mut scope1 = test_scope("openai", "gpt-5-mini");
        let mut scope2 = test_scope("openai", "gpt-5-mini");
        scope1.base_url = "https://api1.com".into();
        scope2.base_url = "https://api2.com".into();

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("m1".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope1);
        r1.try_rename(&req("a", "const a = 1;"));

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("m2".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope2);
        assert_eq!(
            r2.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("m1".into())
        );
        assert_eq!(count2.load(SeqCst), 0);
    }

    #[test]
    fn different_json_mode_is_a_hit() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let mut scope1 = test_scope("openai", "gpt-5-mini");
        let mut scope2 = test_scope("openai", "gpt-5-mini");
        scope1.json_mode = "ladder".into();
        scope2.json_mode = "prompt".into();

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("m1".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope1);
        r1.try_rename(&req("a", "const a = 1;"));

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("m2".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope2);
        assert_eq!(
            r2.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("m1".into())
        );
        assert_eq!(count2.load(SeqCst), 0);
    }

    #[test]
    fn different_provider_is_a_hit() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope1 = test_scope("openai", "model");
        let scope2 = test_scope("anthropic", "model");

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("m1".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope1);
        r1.try_rename(&req("a", "const a = 1;"));

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("m2".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope2);
        assert_eq!(
            r2.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("m1".into())
        );
        assert_eq!(count2.load(SeqCst), 0);
    }

    /// The key is the code being asked about, nothing else. Guards against a
    /// future change quietly reintroducing a partitioning field: any such field
    /// would make this assertion fail.
    #[test]
    fn key_covers_only_identifier_and_normalized_context() {
        assert_eq!(
            cache_key("a", "const a = 1;"),
            sha256_hex_parts(&[CACHE_FORMAT.as_bytes(), b"a", b"const a=1"]),
        );
    }

    // --- --cache-context-size: the key window is not the prompt window ---

    /// The key follows `cache_context` alone. Whatever the model was shown is
    /// irrelevant to filing, which is what lets two `--context-size` values meet.
    #[test]
    fn key_is_built_from_the_cache_context_not_the_prompt_context() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner, _) = CountingRenamer::new(RenameOutcome::Ok("sum".into()));
        let mut r = CachingRenamer::new(Box::new(inner), cache.clone(), scope);
        r.try_rename(&keyed_req(
            "c",
            "a very wide prompt window: var c=a+b",
            "var c=a+b",
        ));

        // Filed under the narrow window, so a plain lookup on that window finds it.
        let key = cache_key("c", "var c=a+b");
        let sha = cache_context_fingerprint("var c=a+b");
        assert_eq!(cache.get(&key, "c", &sha).as_deref(), Some("sum"));
    }

    /// The regression this whole design turns on. Two runs at different
    /// `--context-size` but the same `--cache-context-size` produce the *same
    /// key*. If the stored verification hash were taken from the prompt window
    /// instead of the key window, the second run would not merely miss — `get`
    /// deletes an entry that fails to verify, so the two runs would erase each
    /// other's work on every identifier, making the flag worse than useless.
    #[test]
    fn runs_at_different_prompt_sizes_share_entries_without_destroying_them() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "expensive-model");
        let key_window = "var c=a+b";

        // Expensive pass: wide prompt window, narrow key window.
        let (inner1, count1) = CountingRenamer::new(RenameOutcome::Ok("sum".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope.clone());
        r1.try_rename(&keyed_req(
            "c",
            "function f(a,b){var c=a+b;return c}",
            key_window,
        ));
        assert_eq!(count1.load(SeqCst), 1);

        // Cheap pass: narrow prompt window, same key window.
        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache.clone(), scope.clone());
        assert_eq!(
            r2.try_rename(&keyed_req("c", "var c=a+b;return c", key_window)),
            RenameOutcome::Ok("sum".into()),
            "the cheap pass must reuse the expensive pass's answer"
        );
        assert_eq!(count2.load(SeqCst), 0, "no second call should be made");

        // And the entry is still there afterwards. A verification mismatch would
        // have deleted it, which a hit-count assertion alone would not catch.
        let (inner3, count3) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
        let mut r3 = CachingRenamer::new(Box::new(inner3), cache, scope);
        assert_eq!(
            r3.try_rename(&keyed_req("c", "a third, different window", key_window)),
            RenameOutcome::Ok("sum".into()),
            "the entry must survive being read by a run with a different prompt window"
        );
        assert_eq!(count3.load(SeqCst), 0);
    }

    /// `BudgetRenamer` sits between the walker and `CachingRenamer`. If it
    /// rebuilt the request instead of forwarding it, the key window would be
    /// silently replaced by the prompt window and every key would be wrong.
    #[test]
    fn the_decorator_chain_preserves_the_cache_context() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner, _) = CountingRenamer::new(RenameOutcome::Ok("named".into()));
        let budget = BudgetRenamer::new(Box::new(inner), None);
        let mut caching = CachingRenamer::new(Box::new(budget), cache.clone(), scope);
        caching.try_rename(&keyed_req("c", "wide prompt window", "narrow"));

        assert_eq!(
            cache
                .get(
                    &cache_key("c", "narrow"),
                    "c",
                    &cache_context_fingerprint("narrow")
                )
                .as_deref(),
            Some("named"),
            "the key window must survive the decorator chain intact"
        );
    }

    /// Provenance for the "why did these two runs not share entries?" question.
    /// Recorded, never checked — an entry written at one pair of sizes is
    /// deliberately served to a run using another.
    #[test]
    fn entry_records_both_context_sizes() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner, _) = CountingRenamer::new(RenameOutcome::Ok("named".into()));
        let mut r =
            CachingRenamer::new(Box::new(inner), cache, scope).with_context_sizes(2000, 300);
        r.try_rename(&keyed_req("a", "const a = 1;", "const a = 1;"));

        let key = cache_key("a", "const a = 1;");
        let path = tmp
            .path()
            .join(CACHE_FORMAT)
            .join(&key[..2])
            .join(format!("{key}.json"));
        let entry: CacheEntry = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(entry.context_size, Some(2000));
        assert_eq!(entry.cache_context_size, Some(300));
    }

    /// An entry written before these fields existed must still parse. `get`
    /// deletes anything it cannot deserialize, so a missing `serde(default)`
    /// here would quietly re-buy every name in a warm cache.
    #[test]
    fn an_entry_without_recorded_sizes_still_parses() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let sha = cache_context_fingerprint("const a = 1;");

        // Serialize a full entry, then strip the two newer fields the way an
        // older build's file would look.
        let entry = CacheEntry::new(&key, "named", "a", &sha, &scope);
        let mut json: serde_json::Value = serde_json::to_value(&entry).unwrap();
        let obj = json.as_object_mut().unwrap();
        obj.remove("context_size");
        obj.remove("cache_context_size");

        let shard_dir = cache.root().join(&key[..2]);
        fs::create_dir_all(&shard_dir).unwrap();
        fs::write(
            shard_dir.join(format!("{key}.json")),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();

        assert_eq!(cache.get(&key, "a", &sha).as_deref(), Some("named"));
    }

    // --- key normalization ---

    #[test]
    fn normalizes_whitespace_and_semicolon_runs() {
        assert_eq!(normalize_for_key("const   a\t=\n\n1 ;"), "const a=1");
        assert_eq!(normalize_for_key("a;;;b"), "a b");
        assert_eq!(normalize_for_key("  lead and trail  "), "lead and trail");
    }

    #[test]
    fn normalizes_spacing_around_punctuation() {
        assert_eq!(normalize_for_key("f( a , b )"), "f(a,b)");
        assert_eq!(normalize_for_key("{ a : 1 }"), "{a:1}");
        assert_eq!(normalize_for_key("x = y + z"), "x=y+z");
    }

    /// The load-bearing guard: a space between two identifier characters is a
    /// token boundary and must survive, or `return x` would hash as `returnx`.
    #[test]
    fn keeps_separators_between_identifier_characters() {
        assert_eq!(normalize_for_key("return  x"), "return x");
        assert_eq!(normalize_for_key("typeof\tx"), "typeof x");
        assert_eq!(normalize_for_key("new   Foo()"), "new Foo()");
        assert_eq!(normalize_for_key("a instanceof B"), "a instanceof B");
        assert_eq!(normalize_for_key("case 1:"), "case 1:");
        assert_ne!(normalize_for_key("return x"), normalize_for_key("returnx"));
    }

    #[test]
    fn folds_quote_style_but_keeps_the_quotes() {
        assert_eq!(normalize_for_key("a='x'"), normalize_for_key("a=\"x\""));
        assert_ne!(
            normalize_for_key("const a = \"userId\""),
            normalize_for_key("const a = userId"),
            "a string literal and a bare identifier must not collide"
        );
    }

    /// The headline case: the same code minified and reformatted produces one
    /// cache key, so a formatted bundle reuses a minified run's answers.
    #[test]
    fn minified_and_formatted_code_share_a_key() {
        let minified = "function f(a,b){var c=a+b;return c}";
        let formatted = "function f(a, b) {\n    var c = a + b;\n    return c;\n}\n";
        assert_eq!(normalize_for_key(minified), normalize_for_key(formatted));
        assert_eq!(cache_key("c", minified), cache_key("c", formatted));
        assert_eq!(
            cache_context_fingerprint(minified),
            cache_context_fingerprint(formatted)
        );
    }

    /// Reformatting the input must produce a *hit*, not a delete-and-refetch.
    /// `DiskCache::get` removes an entry whose stored hash fails to verify, so
    /// this fails loudly if the key and the fingerprint ever normalize differently.
    #[test]
    fn a_reformatted_context_hits_the_minified_entry() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("sum".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope.clone());
        r1.try_rename(&req("c", "function f(a,b){var c=a+b;return c}"));

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope);
        assert_eq!(
            r2.try_rename(&req(
                "c",
                "function f(a, b) {\n    var c = a + b;\n    return c;\n}\n"
            )),
            RenameOutcome::Ok("sum".into())
        );
        assert_eq!(count2.load(SeqCst), 0, "reformatting must not cost a call");
        assert_eq!(r2.stats().hits.load(SeqCst), 1);
    }

    #[test]
    fn genuinely_different_code_still_gets_different_keys() {
        assert_ne!(
            cache_key("a", "const a = getUserName();"),
            cache_key("a", "const a = getUserAge();")
        );
        assert_ne!(
            cache_key("a", "const a = 1;"),
            cache_key("b", "const a = 1;")
        );
    }

    /// Provenance survives even though neither the model nor the prompt shape
    /// partitions the key any more: an entry still records both.
    #[test]
    fn entry_records_the_model_that_produced_it() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope1 = test_scope("openai", "expensive-model");

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("goodName".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope1);
        r1.try_rename(&req("a", "const a = 1;"));

        let key = cache_key("a", "const a = 1;");
        let path = tmp
            .path()
            .join(CACHE_FORMAT)
            .join(&key[..2])
            .join(format!("{key}.json"));
        let entry: CacheEntry = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(entry.model, "expensive-model");
        assert_eq!(entry.provider, "openai");
        assert_eq!(
            entry.prompt_fingerprint,
            crate::llm::renamer::prompt_fingerprint(),
            "the request shape is still recorded even though it no longer keys the entry"
        );
    }

    #[test]
    fn refresh_ignores_a_stored_answer_but_overwrites_it() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        // Seed the cache with an answer from a normal run.
        let (inner1, _) = CountingRenamer::new(RenameOutcome::Ok("stale".into()));
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope.clone());
        r1.try_rename(&req("a", "const a = 1;"));

        // --refresh-cache: the stored answer is ignored and the model is re-asked.
        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("fresh".into()));
        let mut r2 =
            CachingRenamer::new(Box::new(inner2), cache.clone(), scope.clone()).with_refresh(true);
        assert_eq!(
            r2.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("fresh".into())
        );
        assert_eq!(count2.load(SeqCst), 1);
        assert_eq!(r2.stats().bypassed.load(SeqCst), 1);
        assert_eq!(
            r2.stats().hits.load(SeqCst),
            0,
            "a bypassed lookup is not a hit"
        );
        assert_eq!(
            r2.stats().misses.load(SeqCst),
            0,
            "a bypassed lookup is not a miss either — the cache was never consulted"
        );

        // The refreshed answer replaced the stale one for subsequent normal runs.
        let (inner3, count3) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
        let mut r3 = CachingRenamer::new(Box::new(inner3), cache, scope);
        assert_eq!(
            r3.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Ok("fresh".into())
        );
        assert_eq!(count3.load(SeqCst), 0);
    }

    /// `--refresh-cache` still writes; `--no-cache` (no `CachingRenamer` at all)
    /// writes nothing. Guards against the two being conflated.
    #[test]
    fn refresh_still_writes() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner, _) = CountingRenamer::new(RenameOutcome::Ok("fresh".into()));
        let mut r = CachingRenamer::new(Box::new(inner), cache, scope).with_refresh(true);
        r.try_rename(&req("a", "const a = 1;"));
        assert_eq!(r.stats().writes.load(SeqCst), 1);
    }

    #[test]
    fn failed_call_is_not_cached() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner1, count1) = CountingRenamer::new(RenameOutcome::Failed {
            reason: "err".into(),
        });
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope.clone());
        let res1 = r1.try_rename(&req("a", "const a = 1;"));
        assert_eq!(
            res1,
            RenameOutcome::Failed {
                reason: "err".into()
            }
        );
        assert_eq!(count1.load(SeqCst), 1);

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("success".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope);
        let res2 = r2.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res2, RenameOutcome::Ok("success".into()));
        assert_eq!(count2.load(SeqCst), 1);
    }

    #[test]
    fn skipped_call_is_not_cached() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        let (inner1, _) = CountingRenamer::new(RenameOutcome::Skipped {
            reason: "budget".into(),
        });
        let mut r1 = CachingRenamer::new(Box::new(inner1), cache.clone(), scope.clone());
        r1.try_rename(&req("a", "const a = 1;"));

        let (inner2, count2) = CountingRenamer::new(RenameOutcome::Ok("success".into()));
        let mut r2 = CachingRenamer::new(Box::new(inner2), cache, scope);
        let res2 = r2.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res2, RenameOutcome::Ok("success".into()));
        assert_eq!(count2.load(SeqCst), 1);
    }

    #[test]
    fn corrupt_entry_is_a_miss() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let shard = &key[..2];
        let shard_dir = cache.root.join(shard);
        fs::create_dir_all(&shard_dir).unwrap();
        let file_path = shard_dir.join(format!("{key}.json"));
        fs::write(&file_path, "{ corrupted json").unwrap();

        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("recovered".into()));
        let mut r = CachingRenamer::new(Box::new(inner), cache, scope);
        let res = r.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res, RenameOutcome::Ok("recovered".into()));
        assert_eq!(count.load(SeqCst), 1);
    }

    #[test]
    fn entry_with_mismatched_original_is_a_miss() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let cache_context_sha = cache_context_fingerprint("const a = 1;");
        let mut entry = CacheEntry::new(&key, "wrong", "b", &cache_context_sha, &scope);
        entry.original = "tampered".into();
        cache.put(&entry).unwrap();

        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("good".into()));
        let mut r = CachingRenamer::new(Box::new(inner), cache, scope);
        let res = r.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res, RenameOutcome::Ok("good".into()));
        assert_eq!(count.load(SeqCst), 1);
    }

    #[test]
    fn order_independence() {
        let tmp = TempDir::new().unwrap();
        let scope = test_scope("openai", "gpt-5-mini");

        // Run 1: rename A then B
        {
            let cache = DiskCache::open(tmp.path()).unwrap();
            let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("renamed".into()));
            let mut r = CachingRenamer::new(Box::new(inner), cache, scope.clone());
            assert_eq!(
                r.try_rename(&req("a", "const a = 1;")),
                RenameOutcome::Ok("renamed".into())
            );
            assert_eq!(
                r.try_rename(&req("b", "const b = 2;")),
                RenameOutcome::Ok("renamed".into())
            );
            assert_eq!(count.load(SeqCst), 2);
        }

        // Run 2: rename B then A in fresh instance over same dir
        {
            let cache = DiskCache::open(tmp.path()).unwrap();
            let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
            let mut r = CachingRenamer::new(Box::new(inner), cache, scope);
            assert_eq!(
                r.try_rename(&req("b", "const b = 2;")),
                RenameOutcome::Ok("renamed".into())
            );
            assert_eq!(
                r.try_rename(&req("a", "const a = 1;")),
                RenameOutcome::Ok("renamed".into())
            );
            assert_eq!(count.load(SeqCst), 0);
            assert_eq!(r.stats().hits.load(SeqCst), 2);
        }
    }

    /// A hand-edited or colliding entry must not poison its key forever. On
    /// Windows `fs::rename` refuses an existing destination, so a naive `put`
    /// would leave the bad file in place and re-pay for the same LLM call on
    /// every future run. Regression guard: miss -> rewrite -> hit.
    #[test]
    fn mismatched_entry_is_repaired_and_then_hits() {
        let tmp = TempDir::new().unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let cache_context_sha = cache_context_fingerprint("const a = 1;");

        // Plant an entry that parses but answers a different question.
        {
            let cache = DiskCache::open(tmp.path()).unwrap();
            let mut bad = CacheEntry::new(&key, "wrongName", "a", &cache_context_sha, &scope);
            bad.original = "tampered".into();
            cache.put(&bad).unwrap();
        }

        // First run: miss, calls the model, overwrites the bad entry.
        {
            let cache = DiskCache::open(tmp.path()).unwrap();
            let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("goodName".into()));
            let mut r = CachingRenamer::new(Box::new(inner), cache, scope.clone());
            assert_eq!(
                r.try_rename(&req("a", "const a = 1;")),
                RenameOutcome::Ok("goodName".into())
            );
            assert_eq!(count.load(SeqCst), 1);
            assert_eq!(r.stats().writes.load(SeqCst), 1);
            assert_eq!(r.stats().write_errors.load(SeqCst), 0);
        }

        // Second run: the repair must have stuck, or the key is still poisoned.
        {
            let cache = DiskCache::open(tmp.path()).unwrap();
            let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("unused".into()));
            let mut r = CachingRenamer::new(Box::new(inner), cache, scope);
            assert_eq!(
                r.try_rename(&req("a", "const a = 1;")),
                RenameOutcome::Ok("goodName".into())
            );
            assert_eq!(count.load(SeqCst), 0, "second run must hit the cache");
        }
    }

    /// A stale entry whose answer differs (a re-run after a prompt tweak that
    /// didn't move the fingerprint, say) is replaced rather than silently kept.
    #[test]
    fn put_replaces_an_entry_with_a_different_answer() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let sha = cache_context_fingerprint("const a = 1;");

        cache
            .put(&CacheEntry::new(&key, "first", "a", &sha, &scope))
            .unwrap();
        cache
            .put(&CacheEntry::new(&key, "second", "a", &sha, &scope))
            .unwrap();

        assert_eq!(cache.get(&key, "a", &sha).unwrap(), "second");
    }

    #[test]
    fn unwritable_cache_dir_reports_an_error() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("a_file");
        fs::write(&file_path, "not a directory").unwrap();
        let bad_dir = file_path.join("sub");
        // The caller (`run_preset`) turns this into a warning and runs without a
        // cache; `cli_smoke::unwritable_cache_dir_still_produces_output` covers
        // that end to end.
        assert!(DiskCache::open(&bad_dir).is_err());
    }

    /// Genuinely concurrent, from threads racing on one key. Both writers must
    /// return `Ok`, exactly one file must survive, and it must be readable.
    #[test]
    fn concurrent_put_of_same_key_is_ok() {
        let tmp = TempDir::new().unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let cache_context_sha = cache_context_fingerprint("const a = 1;");

        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let dir = tmp.path().to_path_buf();
                let scope = scope.clone();
                let key = key.clone();
                let sha = cache_context_sha.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let cache = DiskCache::open(&dir).unwrap();
                    // Half the writers propose a different name, so the
                    // replacement path races too, not just the equal-entry path.
                    let name = if i % 2 == 0 { "alpha" } else { "beta" };
                    let entry = CacheEntry::new(&key, name, "a", &sha, &scope);
                    barrier.wait();
                    cache.put(&entry)
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap().expect("concurrent put must not error");
        }

        let shard_dir = tmp.path().join(CACHE_FORMAT).join(&key[..2]);
        let files: Vec<_> = fs::read_dir(&shard_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files.len(), 1, "one entry, no .tmp debris: {files:?}");

        let cache = DiskCache::open(tmp.path()).unwrap();
        let got = cache.get(&key, "a", &cache_context_sha).unwrap();
        assert!(got == "alpha" || got == "beta", "got {got}");
    }

    /// `get`/`put` are public and slice the key to build a path. Anything that
    /// isn't 64 lowercase hex chars must be rejected, not panic on a non-char
    /// boundary or escape the cache root.
    #[test]
    fn invalid_keys_are_rejected_without_panicking() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let sha = cache_context_fingerprint("x");

        for bad in [
            "",
            "a",
            "é9",                                                               // multi-byte lead
            "../../../../etc/passwd",                                           // traversal
            "ZZ00000000000000000000000000000000000000000000000000000000000000",  // non-hex
            "AB00000000000000000000000000000000000000000000000000000000000000",  // uppercase
            "ab0000000000000000000000000000000000000000000000000000000000000",   // 63 chars
        ] {
            assert_eq!(cache.get(bad, "a", &sha), None, "get({bad:?})");
            let entry = CacheEntry::new(bad, "n", "a", &sha, &scope);
            assert!(cache.put(&entry).is_err(), "put({bad:?}) should be refused");
        }

        // Nothing escaped the root: only the format directory exists.
        let roots: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(roots.len(), 1, "unexpected entries: {roots:?}");
    }

    #[test]
    fn budget_in_the_past_skips_everything_and_still_emits_output() {
        use crate::rename::rename_all_identifiers;

        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("called".into()));
        let past_deadline = Instant::now() - Duration::from_secs(1);
        let mut budget = BudgetRenamer::new(Box::new(inner), Some(past_deadline));

        // Whole pipeline, not just the decorator: the walker must run to
        // completion and Codegen must still emit a valid file.
        let src = "const a = 1; let b = 2; var c = 3;";
        let out = rename_all_identifiers(src, &mut budget, 500).unwrap();
        assert!(out.contains("const a = 1;"), "{out}");
        assert!(out.contains("let b = 2;"), "{out}");
        assert!(out.contains("var c = 3;"), "{out}");
        assert_eq!(count.load(SeqCst), 0, "no calls past the deadline");
        assert_eq!(budget.skipped_count(), 3);
    }

    #[test]
    fn budget_skip_uses_the_shared_reason() {
        let (inner, _) = CountingRenamer::new(RenameOutcome::Ok("called".into()));
        let past = Instant::now() - Duration::from_secs(1);
        let mut budget = BudgetRenamer::new(Box::new(inner), Some(past));
        assert_eq!(
            budget.try_rename(&req("a", "const a = 1;")),
            RenameOutcome::Skipped {
                reason: BUDGET_EXHAUSTED_REASON.to_string()
            }
        );
    }

    #[test]
    fn no_budget_calls_everything() {
        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("called".into()));
        let mut budget = BudgetRenamer::new(Box::new(inner), None);

        let res = budget.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res, RenameOutcome::Ok("called".into()));
        assert_eq!(count.load(SeqCst), 1);
        assert_eq!(budget.skipped_count(), 0);
    }

    #[test]
    fn cache_hits_are_served_after_the_deadline() {
        let tmp = TempDir::new().unwrap();
        let cache = DiskCache::open(tmp.path()).unwrap();
        let scope = test_scope("openai", "gpt-5-mini");
        let key = cache_key("a", "const a = 1;");
        let cache_context_sha = cache_context_fingerprint("const a = 1;");
        let entry = CacheEntry::new(&key, "cachedVal", "a", &cache_context_sha, &scope);
        cache.put(&entry).unwrap();

        let (inner, count) = CountingRenamer::new(RenameOutcome::Ok("live".into()));
        let past_deadline = Instant::now() - Duration::from_secs(1);
        let budget = BudgetRenamer::new(Box::new(inner), Some(past_deadline));
        let mut caching = CachingRenamer::new(Box::new(budget), cache, scope);

        let res = caching.try_rename(&req("a", "const a = 1;"));
        assert_eq!(res, RenameOutcome::Ok("cachedVal".into()));
        assert_eq!(count.load(SeqCst), 0);
        assert_eq!(caching.stats().hits.load(SeqCst), 1);
    }
}
