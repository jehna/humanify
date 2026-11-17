# Module trees

## Command-line interface

```text
Command-line interface
├── Subcommand per Preset
├── Presets
│   ├── OpenAI preset
│   ├── Gemini preset
│   ├── Anthropic preset
│   ├── Ollama preset
│   ├── OpenRouter preset
│   └── Requesty preset
├── Preset runner
│   ├── Configuration resolver — command-line flags over environment over Preset defaults
│   ├── Configuration report
│   └── Progress reporter
└── Input and output pipe
```

## Rename engine

```text
Rename engine
├── Identifier walker — Renames every Identifier, largest Scope first
│   └── Surrounding code window
├── Safe name — turns a Descriptive name into a valid, non-reserved identifier
└── Collision resolver
```

## Language model client

```text
Language model client
├── Language model renamer — keeps the original name when no Descriptive name comes back
├── Answer format ladder — tries Answer formats in order and keeps the first that works
├── Answer formats
│   ├── Chat completions answer formats
│   │   ├── JSON schema answer format
│   │   ├── Forced tool call answer format
│   │   ├── Tool call and prompt answer format
│   │   └── Prompt answer format
│   └── Anthropic answer formats
│       ├── Anthropic native answer format
│       └── Anthropic tool call answer format
└── HTTP transport — tells unsupported Answer formats apart from passing failures
```

## Files and standard streams

```text
Files and standard streams
```

## Chat completions API of a Provider

```text
Chat completions API of a Provider
```

## Anthropic Messages API

```text
Anthropic Messages API
```
