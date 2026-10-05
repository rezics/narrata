# Program fixtures

`hello-v0.nar.hex` (Program format 0, text inline) and `hello-v1.nar.hex` (format 1, content
references; [ADR 0018](../../docs/adr/0018-stage-6-text-free-flow-format.md)) are textual transports of
exact canonical Program envelopes. The CLI accepts `.hex` repository fixtures directly; production
embedders pass the decoded bytes to `load_program`.

