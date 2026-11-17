# Concepts

```text
Minified code — JavaScript whose Identifiers no longer tell what they mean
└── Identifier — a name the code declares for a variable, function, parameter or class
    ├── Scope — the part of the code where an Identifier can be seen
    └── Surrounding code — the code of an Identifier's Scope, cut to the Context size
Readable code — the Minified code with Descriptive names and nothing else changed
Rename — giving one Identifier a Descriptive name everywhere it is used
├── Descriptive name — a name that tells what an Identifier holds or does
├── Collision — a Descriptive name that is already taken where the Identifier can be seen
└── Progress — how many Identifiers have had their Rename out of all of them
Language model — an AI that suggests a Descriptive name from Surrounding code
├── Context size — how much Surrounding code the Language model sees for one Identifier
├── Answer format — the structured form the Language model is asked to answer in
└── Provider — a service that runs Language models
    └── Preset — a Provider with its default Language model, address and key
```
