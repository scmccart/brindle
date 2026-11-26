# Proposal

**Depends on:** `agents-profiles` (the agent zone, its permission rows and notifications), `agents-markdown` and `agents-login`, implemented and archived, and `agents-session-title`, archived first: this change's notification summary names the zone's title that change defines, and both change "Agent notifications", this change's text replacing that one's. It changes no wire message, so it can land before or after `agents-tool-content` and `agents-plans`; design decision 11 says how it meets each.
**Next:** nothing depends on it. `agents-plans` may reuse its `Loader` for an in-progress step.

## Why

The user's manual checks on Windows (2026-11-25) found three ways an agent zone fails to get attention when it needs it:
- Nothing in the conversation shows that the agent is working. Between the user's message and the first chunk, or between tool calls, the zone looks idle. The only signs are a static status dot and the stop button.
- A permission request is drawn like any other card, in the same colours as a message. Allowing a command doesn't look like a risk.
- A permission notification gives the agent's name twice and the tool call's title, and nothing else (`@zed-industries/claude-code-acp — @zed-industries/claude-code-acp`, body `Write /p/hello.txt`). It doesn't say what kind of action is asked or what the options are.

The user chose each remedy on 2026-11-25: an ai-elements Shimmer-style working row and a spinner in the zone header, shadcn's destructive Alert style for permissions, and a notification that gives the context.

## What Changes

- **Working row.** While a turn runs, the conversation ends with a muted status row: `Working…`, or the title of the tool call in progress, or `Stopping…` while a cancel is on its way. Its text shimmers: a lighter band moves across it. It goes away when the turn ends and is never part of the transcript.
- **Spinner.** While an agent zone's status is `working`, its status dot is drawn as a small spinner: on the zone header when the tab has two or more zones, and on the tab's face when the zone's `working` is what the face shows. So the state shows while the conversation is scrolled away and while the tab is in the background.
- **Animation stays cheap.** Every spinner and shimmer in the window is driven by one stepped clock at 10 steps a second, which runs only while some agent zone is working and stops when none is. No `request_animation_frame` loop redraws the window at the display's rate.
- **Permission rows look dangerous.** A pending permission request is drawn as shadcn's destructive alert: a warning icon, a background tinted with the theme's destructive colour, and a destructive border. Its reject options come first and its allow options are destructive buttons. Tab from the prompt input reaches the first reject option first. An answered request goes back to the plain style with its outcome. A request still doesn't take focus when it arrives.
- **Permission notifications give the context.** The summary names the profile and, when it differs, the zone's title: `Claude needs permission — Hello.txt file creation`. The body says what is asked, with its tool kind (`Run: cargo test`), then the options in the row's order (`Reject · Allow once · Always allow`), and `+N more` when several requests wait. The same lines go to D-Bus and to Windows toasts. Done and error notifications use the same names. The tab's title is added only when the user renamed the tab.
- **New `brindle-ui` components**, each with a gallery entry: `Loader` (a stepped `loader-circle` spinner), `Shimmer` (text with a moving highlight), and a destructive tone for `Confirmation`.
- **The dump** gains `--- working`, `--- spinner` and `--- motion` lines.

Not in this change: showing the command or file a permission is about beyond its title (`agents-tool-content` keeps a tool call's raw input and command; decision 8 says how the two meet); answering a permission from the notification's buttons; a reduced-motion setting; spinners for terminal programs' OSC 7501 `working` records.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-zones`: "Permission rows" (reject options first, the destructive style); new requirements for the working row, the working spinner and permission rows' appearance.
- `desktop-notifications`: "Agent notifications" (the summary's names and the permission body).
- `ui-components`: "Confirmations" (the destructive tone); new requirements for loaders and shimmering text.
- `ui-theme-tokens`: "Legible token pairs" gains the tinted permission background.
- `command-line`: "Debug hooks in an agent zone" (the `--- working` line), "Driving an agent zone" (the new tab order).

## Impact

- **`brindle-ui`** (`ui/`): `components/loader.rs` and `components/shimmer.rs` (new), `Confirmation::tone`, the pure shimmer band and spinner step functions, gallery entries and `ui/README.md`'s list. No new dependency.
- **`brindle`** (`src/`): `src/motion.rs` (new, the stepped clock: a GPUI global and its pure step decision); `src/agent/model.rs` (`working_row`, the permission option order, the notice's parts); `src/agent/view.rs` (the working row, the permission row's tone and order); `src/zone_view.rs` and `src/workspace.rs` (the spinner in place of the working dot); `src/notify/policy.rs` (the agent summary), `src/notify/toast.rs` (a body line per `<text>`); `src/theme.rs`'s legibility test.
- **No wire, transcript, config or state file change.**
- **Tests:** unit tests for every pure decision, entity tests in `TestAppContext` for the clock starting and stopping, gallery checks, and e2e cases `agent-working`, `agent-permission-style` and an extended `agent-notify`; `agent-permission-keys`' expectations change with the new order.
- **Docs:** `docs/code-map.md` (`agent/`, `notify/`, `brindle-ui`), `ui/README.md`, CLAUDE.md's dump description.
