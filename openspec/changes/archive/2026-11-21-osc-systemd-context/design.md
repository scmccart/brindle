# Design

## Context

UAPI.15 (OSC 3008, version 1.0) defines `start=`/`end=` sequences with free-order fields and recommends limits without fixing them. It says ordinary terminal resets must not affect the context stack, and only a hangup resets it. Brindle's `osc::Scanner` (`brindle-term-core`) collects payloads for a fixed set of OSC numbers (`9`, `777`, `133`, `7501`) up to `MAX_PAYLOAD` (4096) and returns `Found` values in stream order. Those values feed both the client's `Terminal::handle_found` and the server core's `Emulator::feed`, whose state `replay::render` sends to attaching clients. Tab and zone chrome read cached per-terminal values (`Tab::status`) refreshed on `TerminalEvent::Status`.

## Goals / Non-Goals

**Goals:**
- One pure context model shared by the client and the core, so replays never drift (`fcis R6`).
- A badge that is cheap to draw: the label is computed when the stack changes, never in `render` (`process R14`).

**Non-Goals:**
- Using `command` contexts' `exit=` or timing, or `cwd=` for new tabs (OSC 7 already covers directories).
- Recovering from a context whose `end` never arrives (a dropped ssh connection). See Risks.
- Answering anything: OSC 3008 has no query, and Brindle writes nothing back.

## Decisions

### Pure model: `term-core/src/context.rs`

```rust
pub struct ContextId(String);          // 1..=64 bytes, 0x20..=0x7E, decoded
pub enum Kind { Service, Session, Shell, Command, Vm, Container, Elevate, Chpriv, Subcontext, Remote, Boot, App }
pub struct Fields { kind: Option<Kind>, user, hostname, targetuser, targethost, container, vm, cwd, comm: Option<Text> }
pub enum Message { Start(ContextId, Fields), End(ContextId) }
pub fn parse(payload: &[u8]) -> Option<Message>;
pub struct Contexts { stack: Vec<(ContextId, Fields)> }   // index 0 outermost, last = active
impl Contexts {
    pub fn apply(&mut self, m: Message) -> bool;           // changed?
    pub fn clear(&mut self) -> bool;
    pub fn notable(&self) -> Option<Badge>;
}
pub enum BadgeKind { Privileged, Elsewhere }
pub struct Badge { kind: BadgeKind, label: String }        // label ≤ 24 chars, `…` added
```

The stack is a `Vec` because only the active chain is ever open: a start nests under the active context, and an update or end of a context ends everything after it. So "ends its subcontexts" is a `truncate`, and no tree is needed. The UAPI text says older contexts "stay valid in the background", but they can only be updated or ended, and both truncate anyway. `Kind` is closed and matched without a wildcard (`types R1`, `railway R3`). An unknown `type` becomes `None`, not a catch-all variant, so `notable` can't mistake it for a known kind. Fields Brindle doesn't use (`machineid`, `bootid`, `pid`, `pidfdid`, `cmdline`, `sessionid`, the end fields) are parsed past and dropped. Keeping them would only grow replays. `Text` is a decoded, length-checked string (`types R4`, parse at the boundary). The module joins `PURE_MODULES`.

Alternative considered: a tree keyed by id with an explicit active pointer. Rejected: same behaviour under the spec's rules, more states to get wrong.

### Scanner: `3008` joins the wanted numbers

`osc::Scanner`'s wanted set gains `(3008, 4)`, and `finish` returns `Found::Context(context::Message)`. A payload over 4096 bytes already overflows and is dropped, which is the spec's limit. `Found` is matched exhaustively, so the client, the emulator and the fuzz target must each handle it. The fuzz generator gains `3008` payloads: escapes, long values, missing `start=`.

### Lifetime and replay

- Process exit clears the stack, the nearest thing to UAPI's hangup reset. `Found::Reset` and `Found::PromptStart` leave it alone.
- `replay::render` takes `&Contexts` and writes one `3008;start=…` per open context, outermost first, re-escaping `;`, `\` and bytes outside 0x20–0x7E as `\xHH`.
- Because `ESC c` doesn't clear contexts, a reconnecting client must drop its old stack itself. It does so when the replay begins (`REPLAY_START`, which `Terminal` already handles), then rebuilds from the replay. That is the spec's "replace the session's contexts".
- Versions: replays are bytes, so nothing is negotiated (`process R4`). An old client ignores `3008`. A new client on an old core gets an empty stack, as the spec allows. A core restart renders a checkpoint, so the stack survives it and step-down upgrades (`process R1`–`R3`, `R12`). A Brindle crash loses only client state, which reattach restores.

### Shell and UI

`Terminal` keeps `contexts: Contexts` and a cached `badge: Option<Badge>`, recomputed in `handle_found` when `apply` reports a change, then emits `TerminalEvent::Status(None)`. `Workspace` reads the active zone's badge into `Tab::badge` alongside `Tab::status`. `render_tab_bar` and the zone header draw a `brindle_ui` badge after the title, and the title truncates first down to 8 characters (the existing ellipsis pitfall applies: wrap the title in a `min_w_0` box). `BadgeKind::Privileged` maps to the theme's ANSI 1, `Elsewhere` to the muted foreground.

As built: the badge is a new `brindle_ui::Badge` (shadcn's outline badge), placed by `TabTrigger::badge` and `CardHeader::badge`, and coloured by the tab's own theme (`Theme::badge_color`: ANSI 1, or the cards' muted text), as the face is. The two-step give-way can't be had from a `min_w_0` box alone, since gpui's text measures the same at min- and max-content: the title's flex basis is the measured width of its first 8 characters and an ellipsis, it grows up to its whole measured width, and the badge shrinks 1000 times as fast, so the title is cut to 8 characters before the badge is cut, and only then further. `Tab::badge` is also refreshed on the tree's `TitleChanged`, which a change of active zone sends. The event-overload reset (side events dropped while the UI thread was behind) clears the stack with the status records and progress, since a dropped message leaves it stale.

### Testing

- `context.rs` unit tests: every `program-contexts` scenario (escapes, bad ids, bad type, nesting, update ends subcontexts, unknown end, depth 32, label rules and truncation).
- `osc.rs` and `replay.rs` unit tests: split reads, and a render/feed round trip including escaped ids. An `ESC c` inside the stream doesn't clear.
- `--dump-screen-after` prints `--- context id="…" type=… [label="…"]` per open context of the active terminal, outermost first, and `--- badge tab=N kind=privileged|elsewhere label="…"` per tab with a badge, then `--- badge zone=ID …` per zone of a split active tab with one (an addition, so the cases can check which faces and headers show one). The e2e cases `context-badge` (dump plus a pixel sample of the red badge), `context-end`, `context-zones`, and `local-server-context-attach` print the sequences with `printf`, since no e2e runner can rely on `run0` or `systemd-nspawn`.
- Manual check: `run0` on a systemd ≥ 257 machine shows the badge and drops it on exit.

## Risks / Trade-offs

- [A remote host's context outlives a dropped connection, since nobody sends its `end`] → the badge is wrong until the zone's process exits. A follow-up can tie remote contexts to the local shell's next OSC 133 `A`, once we know how systemd's ssh integration behaves when a connection drops.
- [Any program can claim a context, so a badge can be spoofed] → the badge is informational, not a security boundary; README says so. Brindle grants nothing on the strength of a context.
- [Wide labels crowd the strip] → 24-character cap and title-first truncation.
