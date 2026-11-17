# User stories

- S1: I want to turn a file of Minified code into Readable code so that I can understand what the code does.
- S2: I want to read Minified code from standard input and write Readable code to standard output or a file so that I can chain humanify with other tools, such as an unbundler.
- S3: I want to choose a Preset with a subcommand so that I can use the Provider I have access to without configuring it.
- S4: I want to override the Preset's Language model, address and key so that I can use any Language model of a Provider, or a Provider I host myself.
- S5: I want the key read from the Preset's environment variable when I do not give one so that it does not show in my shell history.
- S6: I want to set the Context size so that I can trade the cost of a Rename for its quality.
- S7: I want the Answer format my Provider supports found automatically so that I do not need to know which ones it supports.
- S8: I want to pin one Answer format so that I can work around a Provider that answers wrongly in the one found automatically.
- S9: I want every use of an Identifier Renamed together, and no other Identifier or global name captured by the new name, so that the Readable code behaves exactly like the Minified code.
- S10: I want a Collision solved with a number at the end of the Descriptive name, while Identifiers in separate Scopes may share a name, so that the Readable code stays valid and the names stay short.
- S11: I want every Descriptive name turned into a valid name that is not a reserved word so that the Readable code always parses.
- S12: I want an Identifier to keep its name when the Language model gives no Descriptive name so that one failure does not stop the whole Rename.
- S13: I want to see the Progress so that I know how long a large file still takes.
- S14: I want to see the resolved configuration and every Rename as it happens so that I can debug a bad result.
- S15: I want to set how long to wait for one answer from the Language model so that a slow local Language model can finish.
- S16: I want distinct exit codes for wrong usage, Minified code that cannot be parsed and failed reading or writing so that my scripts can tell them apart.
