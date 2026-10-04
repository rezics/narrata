# AI agent instructions

Put task-created temporary files in `.temp/`.

Run commands through Task (`task --list`; arguments after `--`). Cargo builds
Rust, npm installs the web example's dependencies, Bun runs repository scripts.
Tools, versions and checks are recorded in `docs/development/toolchain.md`;
update it in the same change that adds or replaces one.

Narrata maintains narrative structure and never owns body content: text, media
and option labels are content references that the host resolves
(`docs/product/decisions.md`). A change to a stored or exchanged format needs an
ADR in `docs/adr/` and a read or migration path; frozen corpora under
`fixtures/compat/` are read-only.

Express in code whatever code can express: types, schemas, tests, lint rules and
comments. Write documents only for what code cannot carry, such as intent,
decisions with their reasons and operating procedures. Documents record practice
to consult, not rules; when one no longer fits, change it and explain why. When
code implements a page under `docs/contracts/`, shrink the page to what the code
cannot say.

Write tests with the implementation and run the checks the change affects:
`task check:r1` for the node stack and web reader, `task check:g5` for the
kernel crates, `task test:scripts` for repository scripts and `task docs:check`
for documentation changes.

Never modify REZICS repositories. Record what Narrata needs from REZICS as
capability requests in `docs/integrations/rezics.md`.

The maintainer may update any documentation at any time with any tool. Treat
those updates as authoritative: detect them, adapt, never revert them silently;
refine them only in a separate, explained commit.

`GOAL.md` lists the active Goals; each runs from its own directory under
`docs/goals/` with one manager. Managers follow `docs/goals/manager.md`, and
their worker processes follow `docs/goals/worker.md`.
