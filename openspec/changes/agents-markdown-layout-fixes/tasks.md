# Tasks

Before starting, check that `agents-profiles`, `agents-markdown` and `agents-session-recovery` are archived, and read design.md's flags F1–F3: if the user has answered any differently, change decisions 1, 3 or 4 and their tests first.

Each test in groups 1–3 is written first and run against today's code; note in the task whether it failed there (it should, except where design.md says the fault wasn't reproduced).

## 1. brindle-ui: the components

- [ ] 1.1 In `ui/src/components/message.rs`, give the assistant side's column `w_full` with no cap and keep the user side's `max_w(relative(0.8))` shrink-to-fit (decision 1); add `TestAppContext` tests that an assistant message in two parts (`short`, then a paragraph wider than the row) gives both bubbles the row's width and the same x, that an assistant `ok` fills a 600 px row, and that a user `ok` is narrower than half the row and ends at its end; verify with `cargo test -p brindle-ui message`.
- [ ] 1.2 In `ui/src/components/code_block.rs`, draw the label with `trigger_label` in its size and muted colour and keep the copy button `flex_none` (decision 3); add tests that, inside an assistant `Message` in a 600 px column, a `rust` label is one line high and as wide as its text, and that a label wider than the row is one line high with the copy button at its full width; verify with `cargo test -p brindle-ui code_block`.
- [ ] 1.3 In `ui/src/components/prompt_input.rs`, give the reason `flex_1().min_w_0()` and a `debug_selector` (`prompt-reason`), the action staying `flex_none` after it (decision 4); add tests that the reason `connecting to the terminal server` at 240 px is at least as wide as `connecting` and at most two lines high, and that a host drawn at 600 px and then at 240 px gives the reason the bounds a fresh 240 px draw gives; verify with `cargo test -p brindle-ui prompt_input`, and that the existing reason-action tests still pass.
- [ ] 1.4 In `ui/src/components/confirmation.rs`, make a stacked action's column `w_full` with `items_start` (the audit's suspect site); add a test that stacked actions with a long description at 240 px wrap it at the confirmation's width with each button at its own width; verify with `cargo test -p brindle-ui confirmation`.
- [ ] 1.5 Re-run the audit's search (`grep -rn min_w_0 ui/src src`) against design.md's table and fix any site added since that breaks the rule, with a test for it; write the "Wrapping text pitfall" into `ui/README.md`'s notes and list the assistant bubble's difference from AI Elements there; verify the README's component list still matches `ui/src/components/`.
- [ ] 1.6 Add gallery checks `message-parts-line-up`, `code-block-label-one-line` and `prompt-reason-wraps` to `ui/examples/gallery.rs`, reading painted bounds as its other bounds checks do; verify with `cargo build -p brindle-ui --example gallery` here, and hand `cargo run -p brindle-ui --example gallery -- --check` under `scripts/with-xvfb` to the run chore (it opens a window).

## 2. The app

- [ ] 2.1 Add a view test to `src/agent/view/tests.rs`: an `AgentView` connecting (the `Zone` fixture) drawn at a zone's full width and then at half of it, the reason (`prompt-reason`) at most two lines high and at least as wide as `connecting` both times; run it first on a temporary branch at `main`, without group 1's fixes, and record whether it reproduced the fault; verify with `cargo test -p brindle agent::view`.
- [ ] 2.2 If 2.1 didn't reproduce the fault, add the same check to `src/zone_view.rs`'s `TestAppContext` fixture, splitting an agent zone with `split_zone` and reading the new zone's reason over its first frames; record the outcome in design.md's "As built" either way; verify with `cargo test -p brindle zone_view`.
- [ ] 2.3 Check that `src/agent/view.rs` needs no change beyond what group 1 gives it (its text sits in stretched columns, per the audit); verify `cargo test -p brindle agent` passes and `cargo clippy --locked --workspace --all-targets -- -D warnings` is clean.

## 3. End to end

- [ ] 3.1 Add the `muted` token's colour for the `e2e` theme to `tests/e2e/brindle.rs`'s `theme` constants, with its derivation as `CHROME_BORDER` has; verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [ ] 3.2 Add the case `agent-markdown-layout` (`needs_fake_agent`) to `tests/e2e/cases/agent.rs` and `cases/mod.rs`: the fake agent replies `Hi.`, a long paragraph and a fenced `rust` block; in the capture, the bubble's right border is at one x on every row it spans, `px_3` inside the card's content edge, and the code block's header is at most twice the label's line height; verify with `BRINDLE_E2E=1 cargo test --test e2e -- --list` showing it, and hand the run under `scripts/with-xvfb` to the run chore.
- [ ] 3.3 Add the manual checks to `CLAUDE.md`'s list, beside the Markdown ones: a reply with a short and a long paragraph and a code block reads as one bubble with a one-line label, and a split agent zone's connecting line reads across the zone, on Linux and on Windows.

## 4. Docs and integration

- [ ] 4.1 Update `docs/code-map.md`'s `brindle-ui` entry with the wrapping-text pitfall beside the ellipsis pitfall, and its `agent/` Markdown bullet with the width rule; verify the entry names `trigger_label` and the `flex_1().min_w_0()` form.
- [ ] 4.2 Run the integration chore: `cargo test --workspace --locked`, the clippy command above, the gallery's `--check` and `BRINDLE_E2E=1 cargo test --test e2e -- agent` under `scripts/with-xvfb` (through `systemd-run --user --scope` when run from inside Brindle); verify all pass and report the gallery and e2e lines for the new checks.

## Workflow follow-up

- Archive the change after review, syncing its delta specs.
