# humanify

> Un-minify JavaScript code using LLMs ("AI")

This tool uses large language models (like ChatGPT, Claude, Gemini, and
locally-hosted Ollama models) to unminify and rename minified JavaScript code.
The LLM only suggests new identifier names; the heavy lifting is done by
[oxc](https://github.com/oxc-project/oxc) at the AST level so the rewritten code
remains structurally identical to the input.

## Version 3 is out! 🎉

v3 highlights compared to v2:

* **Single static binary:** (Rust) — grab a binary from Releases. No Node, no
  npm, no Python.
* **Unix-style I/O**: read from stdin or a file, write to stdout or `-o <file>`.
  Combine easily with a deobfuscator like [webcrack][webcrack].
* **New providers:** Ollama (local), Anthropic and OpenRouter.

Check out the [v3 PR](https://github.com/jehna/humanify/pull/744) for more info.

[webcrack]:https://github.com/j4k0xb/webcrack

### ➡️ Check out the [introduction blog post][blogpost] for in-depth explanation!

[blogpost]: https://thejunkland.com/blog/using-llms-to-reverse-javascript-minification

## Example

Given the following minified code in `splitstring.min.js`:

```javascript
function a(e,t){var n=[];var r=e.length;var i=0;for(;i<r;i+=t){if(i+t<r){n.push(e.substring(i,i+t))}else{n.push(e.substring(i,r))}}return n}
```

Run:

```shell
humanify openai splitstring.min.js -o splitstring.js
```

Result (`splitstring.js`):

```javascript
function splitString(inputString, chunkSize) {
  var chunks = [];
  var stringLength = inputString.length;
  var startIndex = 0;
  for (; startIndex < stringLength; startIndex += chunkSize) {
    if (startIndex + chunkSize < stringLength) {
      chunks.push(inputString.substring(startIndex, startIndex + chunkSize));
    } else {
      chunks.push(inputString.substring(startIndex, stringLength));
    }
  }
  return chunks;
}
```

You can also pipe via stdin:

```shell
cat splitstring.min.js | humanify openai - > splitstring.js
```

To unbundle Webpack output first, pipe through `npx webcrack`:

```shell
npx webcrack < bundle.min.js | humanify openai - -o bundle.js
```

## Note on token usage

🚨 **NOTE:** 🚨

humanify makes one LLM call per identifier in your code. For ChatGPT-class APIs
the cost roughly scales with the number of identifiers and the surrounding
context window (default 500 chars per call). A medium minified file (~500
identifiers) typically costs in the range of $0.10–$1.00 with OpenAI's small
models, free with the Gemini free tier, and free with Ollama or OpenRouter free
models.

For a rough character-count estimate of OpenAI mode:

```shell
echo "$((2 * $(wc -c < yourscript.min.js)))"
```

Using `humanify ollama` is free but slower; quality depends on your local model.
Free OpenRouter models (e.g. `qwen/qwen3-coder:free`) can help with your budget,
but expect them to be heaviy rate limited.

## Getting started

### Installation

The preferred way to install humanify is to download a pre-built binary from the
[latest release](https://github.com/jehna/humanify/releases/latest).

```shell
# macOS (Apple Silicon)
curl -L https://github.com/jehna/humanify/releases/latest/download/humanify-aarch64-apple-darwin.tar.gz | tar xz
sudo mv humanify /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/jehna/humanify/releases/latest/download/humanify-x86_64-apple-darwin.tar.gz | tar xz
sudo mv humanify /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/jehna/humanify/releases/latest/download/humanify-x86_64-unknown-linux-gnu.tar.gz | tar xz
sudo mv humanify /usr/local/bin/

# Linux (aarch64)
curl -L https://github.com/jehna/humanify/releases/latest/download/humanify-aarch64-unknown-linux-gnu.tar.gz | tar xz
sudo mv humanify /usr/local/bin/

# Windows: download humanify-x86_64-pc-windows-msvc.zip from the releases page
```

Or build from source:

```shell
cargo install --git https://github.com/jehna/humanify
```

### Usage

```shell
humanify <openai|gemini|anthropic|ollama|openrouter|requesty> [FLAGS] <INPUT>
```

* `<INPUT>` is a file path or `-` for stdin.
* `-o <FILE>` writes to a file (default: stdout).
* `-m <MODEL>` overrides the preset's default model.
* `-k <KEY>` overrides the env-var-based API key.
* `--base-url <URL>` overrides the preset's base URL.
* `--context-size <N>` sets surrounding-code chars per identifier (default 500).
* `--json-mode <MODE>` pins a JSON-mode strategy. Options:
  `ladder` (default), `openai-json-schema`, `anthropic-native`,
  `forced-tool-call`, `tool-call-and-prompt`, `prompt`.
* `-v` prints resolved configuration and identifier-level rename steps to stderr.
* `--progress` shows an identifier progress bar on stderr.
* `--cache-dir <DIR>` caches every LLM response on disk, keyed by identifier +
  context window (the model is deliberately *not* part of the key). Interrupted
  runs resume for free: re-run with the same `--cache-dir` and only the missing
  identifiers are requested. Also settable via `HUMANIFY_CACHE_DIR`;
  `--no-cache` overrides both.
* `--refresh-cache` ignores stored answers but keeps writing fresh ones, so the
  model is re-asked and the cache is overwritten. Use it to re-do a file with a
  better model, or to A/B two models over the same input. Distinct from
  `--no-cache`, which disables reads *and* writes; it is an error to pass it
  without a cache configured.
* `--cache-context-size <N>` sets the context window used to build the cache
  key, independently of the window sent to the model. Defaults to
  `--context-size`, so leaving it unset preserves existing cache entries. Pin one
  value so that runs at different `--context-size` still share entries. May be
  larger than `--context-size`. Also settable via `HUMANIFY_CACHE_CONTEXT_SIZE`;
  passing the flag without a cache configured is an error.
* `--max-retries <N>` retries transient errors (429 / 5xx / network) with
  exponential backoff (default 3).
* `--max-run-seconds <N>` gives the run a wall-clock budget. When it expires the
  remaining identifiers are left unchanged and the partial output is still
  written; combined with `--cache-dir`, a re-run continues where it stopped.
  Omitted or 0 means unlimited — there is no upper bound.
* `--timeout-seconds <N>` per-request HTTP timeout in seconds (overrides preset default).
* `--max-tokens <N>` caps the response length. Unset by default on hosted APIs.
  Reasoning tokens count against this budget on most providers, so only cap a
  thinking model once thinking is off — otherwise the reply gets truncated to
  nothing and the call fails.
* `--extra-body <JSON>` merges a JSON object into every request body (or
  `@file.json` to read it from a file). Top-level keys override humanify's;
  `messages`, `system` and `stream` are rejected. This is the escape hatch for
  provider-specific parameters humanify has no flag for.
* `--start-sentinel <TEXT>` / `--stop-sentinel <TEXT>` rename only one region of
  the file. See [Renaming one region](#renaming-one-region-sentinels).
* `--sentinel-strict`, `--sentinel-expand-helpers` pick a different selection
  policy for that region.
* `--dry-run` resolves the sentinels, prints the window and the identifiers it
  selects, and exits without making a single LLM call.

Run `humanify --help` for the full reference.

### Reasoning models

A hybrid reasoning model left on its default settings will happily spend
thousands of chain-of-thought tokens choosing a one-word identifier name, which
shows up as occasional multi-minute requests in an otherwise fast run. Turning
thinking off is provider-specific, so it goes through `--extra-body` — for GLM
models on Z.ai:

```shell
humanify openai obfuscated.js \
  --base-url https://api.z.ai/api/coding/paas/v4 -m GLM-4.6 \
  --extra-body '{"thinking":{"type":"disabled"},"temperature":0}' \
  --max-tokens 128
```

Note that the response cache is not keyed on these two flags: entries written
before you changed them are still served afterwards. Add `--no-cache` (or use a
separate `--cache-dir`) when comparing settings.

Note: humanify does one job — rename identifiers in one JavaScript file in,
one out. To unbundle webpack output first, pipe through e.g.
[webcrack](https://github.com/j4k0xb/webcrack):

```shell
npx webcrack < bundle.min.js | humanify openai - -o bundle.js
```

### OpenAI mode

You'll need an OpenAI API key. Sign up at https://openai.com/ and create a
key in the dashboard.

```shell
humanify openai obfuscated.js -o readable.js -k your-token
```

Or via environment variable:

```shell
export OPENAI_API_KEY=your-token
humanify openai obfuscated.js -o readable.js
```

Default model: `gpt-5-mini`. Override with `-m`.

### Gemini mode

You'll need a Google AI Studio key. Sign up at https://aistudio.google.com/.
Gemini's free tier is generous and is enough for most files.

```shell
export GEMINI_API_KEY=your-token
humanify gemini obfuscated.js -o readable.js
```

Default model: `gemini-3.1-flash-lite`. Override with `-m`.

### Anthropic mode

You'll need an Anthropic API key. Sign up at https://console.anthropic.com/.

```shell
export ANTHROPIC_API_KEY=your-token
humanify anthropic obfuscated.js -o readable.js
```

Default model: `claude-sonnet-4-6`. Override with `-m`.

The Anthropic preset uses Anthropic's native structured-outputs API
(`output_format: json_schema`) when available, falling back to forced
tool-calls if your account doesn't have the structured-outputs beta enabled.

### Local mode (Ollama)

Local mode runs against [Ollama](https://ollama.com/), which manages local LLM
weights and exposes an OpenAI-compatible API on `localhost:11434`. (pre-v3
migration note: There's no `humanify download` anymore — use a local inference
provider like Ollama to run your own models)

Prerequisites:
1. Install Ollama: <https://ollama.com/download>
2. Pull the recommended model: `ollama pull qwen3.5:4b`

Then run:

```shell
humanify ollama obfuscated.js -o readable.js
```

Default model: `qwen3.5:4b`. Override with `-m` to use any model you've
pulled. Local mode is free and private, but slower and less accurate than
the hosted providers; quality depends on the model you pick.

If you want to point humanify at a remote Ollama instance, override the
base URL:

```shell
humanify ollama obfuscated.js --base-url http://my-server:11434/v1
```

### OpenRouter mode

[OpenRouter](https://openrouter.ai/) routes requests across many backend
models. Useful for trying free-tier coding models without setting up
multiple accounts.

You'll need an OpenRouter API key. Sign up at https://openrouter.ai/.

```shell
export OPENROUTER_API_KEY=your-token
humanify openrouter obfuscated.js -o readable.js
```

Default model: `openai/gpt-oss-120b`. For free-tier usage:

```shell
humanify openrouter obfuscated.js -m qwen/qwen3-coder:free
```

### Requesty mode

[Requesty](https://requesty.ai/) provides an OpenAI-compatible router across
many backend models via a single API key.

You'll need a Requesty API key. Sign up at https://requesty.ai/.

```shell
export REQUESTY_API_KEY=your-token
humanify requesty obfuscated.js -o readable.js
```

Default model: `nvidia/nemotron-3-super-120b-a12b`. Override with `-m`:

```shell
humanify requesty obfuscated.js -m nvidia/nemotron-3-super-120b-a12b
```

## Renaming one region (sentinels)

Many times you don't want to pay to humanify a whole bundle, you want to
only review one module/function. `--start-sentinel` and `--stop-sentinel` mark that region, and
only the identifiers visible in it are sent to the model.

**A sentinel is a literal fragment of the input file**, resolved by plain text
search.Pick a distinctive-looking run of characters that only occurs once in the file for the sentinel:

```shell
humanify openai bundle.min.js -o bundle.js \
  --start-sentinel 'or(var t=e.split(/\n+/g' \
  --stop-sentinel  'return n.filter(Boolean)'
```

The window runs from the **start** of the start-match to the **end** of the
stop-match, so both fragments are inside it. Either flag may be omitted (start of
file / end of file respectively). The fragment is matched against the raw input
bytes, so copy it from the input file itself. Because you are pointing at code that
is already there, the input file is never edited and every existing cache entry
stays valid.

A fragment that matches **zero** times, or **more than once**, is a hard error
(exit 64) — the multi-match message lists every occurrence as `line:col`.
Silently picking the first match would spend real money on the wrong region, and
since you can pre-verify uniqueness in your editor, a hard error costs you
nothing. Stale markers from a previous session are caught the same way.

### `@file` for awkward fragments

Code fragments contain quotes, parens, backslashes and `$`, which are unpleasant
to quote on a command line — especially in PowerShell, where backtick is the
escape character and `$` interpolates. Both flags accept `@path` and read the
fragment verbatim from a file (one trailing newline is trimmed, since editors add
one):

```shell
humanify openai bundle.min.js --start-sentinel @start.txt --stop-sentinel @stop.txt
```

### Check before you buy: `--dry-run`

`--dry-run` resolves the sentinels, applies the filter, prints the window and the
selected identifiers to stderr, and exits **without constructing an LLM client or
making a single call**. Nothing is written to `-o`.

```
$ humanify openai bundle.min.js --start-sentinel 'function target' --stop-sentinel 'return local; }' --dry-run
humanify: dry run: no LLM calls made, no output written
humanify: sentinel window: bytes 99..161 (lines 3..3), 3 of 6 identifiers selected
humanify: selected identifiers: helperFn, local, target
```

A normal run prints the same `sentinel window:` line without `--verbose`, so a
mis-aimed window is obvious in the first second rather than after the bill.

### What gets renamed

By default, an identifier is renamed if it is **declared in the window or
referenced in the window** — everything visible in the region you are reading. So
a helper declared hundreds of lines earlier but *called* inside the window is
renamed, because the call site then reads `parseColorCodes(e)` instead of
`xue(e)`. Its params and locals are **not**: you aren't going to read a
third-party helper's body, and paying a model to name its loop counters is waste.

| flag | what it selects |
|---|---|
| *(default)* | declaration **or** reference in the window |
| `--sentinel-strict` | declaration in the window only |
| `--sentinel-expand-helpers` | the default, plus the bodies of helpers pulled in by reference |

`--sentinel-expand-helpers` can add quite a bit of cost as any method directly called from within
the sentinel block also has all identifiers looked up.  This deliberately stops after one level
following calls transitively would reach a bundler runtime quickly and end up back at the whole file.

Things that are unaffected by any of this:

* **Collisions.** Symbols outside the window still participate in collision
  detection, so a new name inside the window is still suffixed if it would shadow
  or capture an untouched name outside it.
* **Prompts.** A symbol's context window is always derived from where it is
  *declared*, no matter why it ended up in the work list. A helper pulled in by a
  call site is still described to the model by the code around its own
  declaration.
* **Unrenamed symbols.** They simply print with their original names.


### Mixing costs over one file

The point of all this is a mixed-cost workflow. Because the cache key covers only
the question (see [Caching](#caching)), a sentinel run and a later whole-file run
share answers:

```shell
# Buy the region you care about with a good model
humanify anthropic bundle.min.js --cache-dir .cache \
  --start-sentinel 'function target' --stop-sentinel 'return local; }' -o partial.js

# Sweep the rest cheaply; the region's names are reused, not re-bought
humanify ollama bundle.min.js --cache-dir .cache -o bundle.js
```

Order does not matter — whichever run reaches an identifier first supplies its
name. Use `--refresh-cache` to genuinely replace earlier answers (ie to do the more costly run after the main run).

## Caching

humanify supports persistent on-disk response caching via `--cache-dir <DIR>` or the `HUMANIFY_CACHE_DIR` environment variable. `--no-cache` disables caching even if a directory is set.

When caching is enabled, every successful LLM response is persisted to `<DIR>/humanify-cache-v1/` immediately upon receipt. If a run is interrupted (e.g. killed, Ctrl+C, or hits `--max-run-seconds`), repeating the command reuses all completed responses without making additional network calls.

### What invalidates cache entries
A cache key identifies the *code being asked about*, and nothing else. It is derived from:
- **Identifier name & Context window**: The original identifier name and surrounding context code slice.
- **`--context-size` changes**: Changing `--context-size` alters the context window slices and therefore invalidates existing entries for that file — unless you pin `--cache-context-size` (see below).

### Keying independently of `--context-size`

`--cache-context-size <N>` builds the cache key from its own window while the model still sees the `--context-size` one. Pin it, and an expensive pass at `--context-size 2000` and a cheap pass at `--context-size 300` share every entry instead of each paying in full.

The value matters less than its stability: **pick one and never change it.** A smaller key window yields more hits but a higher chance that a name derived from a narrow neighbourhood is reused where a wide-context run would have produced a better one — the same deliberate reuse trade-off already made by keeping the model out of the key.

Two things worth knowing before choosing a number:

- **The window is the enclosing scope, not a fixed slice around the identifier.** When that scope fits inside the size, the *whole scope* is used and the size is irrelevant. Both the trade-off above and the flag itself therefore only bite for identifiers in scopes larger than the key window — top-level symbols in a minified bundle, and very large module functions. Everything else already shares entries at any size.
- **`N` is counted in bytes**, rounded out to the nearest UTF-8 character boundary, as `--context-size` is.

Each entry records the `context_size` and `cache_context_size` it was written under, so a run that unexpectedly shares nothing can be diagnosed from the entry JSON. Neither is part of the key: an entry written at one pair of sizes is deliberately served to a run using another.

Entries are never invalidated by anything describing *how* the question was asked:
- **The model, provider, base URL, or JSON mode.** See below.
- **The prompt, JSON schema, and request shape** — including humanify upgrades that change them. A warm cache keeps serving its stored answers until you refresh it.
- API keys (secrets never affect cache keys or entries).
- CLI flags like `--timeout-seconds`, `--max-retries`, `--verbose`, or `--progress`.

### Mixing models over one file

Because neither the model nor the prompt is part of the key, answers are shared across runs. That makes a mixed-cost pass possible: run an expensive model over the part of a bundle you actually care about, then a cheap model over the whole file, and the expensive answers are reused instead of being re-bought. Order does not matter — whichever run reaches an identifier first supplies its name.

The trade-off is that one output can contain names from several models or several humanify versions, invisibly. Two ways to take control:

- `--refresh-cache` re-asks the model and overwrites what is stored, so a second run at a better setting genuinely replaces the earlier answers.
- A separate `--cache-dir` gives a run its own namespace — what you want for a clean A/B comparison between models or prompts.

Each entry still records the provider, model, base URL, JSON mode and prompt fingerprint that produced it, so the origin of any cached name remains inspectable in the entry's JSON.

Failures (e.g., network errors, 429 rate limits, malformed responses) and skipped identifiers are **never cached**. The cache directory is safe to delete at any time, and multiple concurrent humanify processes can safely share the same cache directory.

### Reuse across different-but-similar files

Cache entries can be reused across different versions of a bundle or across related files. Because context windows are anchored to the enclosing scope rather than absolute file offsets:
- Adding or removing code elsewhere in a file does not change the scope content of untouched functions.
- Untouched functions reuse cached renames directly.

**Note on re-minified bundles:** The identifier name is part of the cache key because it is interpolated into the prompt. If a file is re-minified with newly scrambled/reassigned variable names (e.g. `function(a,e,t)` changed to `function(e,t,n)`), the prompt and context bytes differ, reducing cache reuse even if the underlying code logic is unchanged. Stable identifier assignment or hand-edited bundles achieve high cache reuse.

## Features

* Uses LLMs to get smart suggestions to rename variable and function names, and
  make the rename using deterministic AST-level shenanigans via [oxc][oxc]
* Renames preserve all references and respect lexical scoping
* Reserved-word and collision-aware safe naming. The LLM's suggestion is
  normalised to a valid JS identifier and `_`-prefixed if it collides with
  an existing binding

[oxc]:https://github.com/oxc-project/oxc

## Contributing

If you'd like to contribute, please fork the repository and use a feature
branch. Pull requests are warmly welcome.

```shell
git clone https://github.com/jehna/humanify
cd humanify
cargo build
cargo test
```

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test` on
every PR. Local `ollama` and judge e2e suites also run on every PR.
The `gemini` e2e suite runs by default only for branches in this repository.
Other providers' e2e suites require both a branch in this repository and
their corresponding label (`test-openai`, `test-anthropic`,
`test-openrouter`, `test-requesty`) to avoid exposing secrets to forks or
burning API credits on every PR.

## Star History

<a href="https://www.star-history.com/?repos=jehna%2Fhumanify&type=date&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=jehna/humanify&type=date&theme=dark&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=jehna/humanify&type=date&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=jehna/humanify&type=date&legend=top-left" />
 </picture>
</a>

## Licensing

The code in this project is licensed under MIT license.
