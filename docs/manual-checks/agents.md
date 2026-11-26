# Agent zones: manual checks

These are the checks the e2e suite can't make for agent zones. They cover the agent stack merged on 2026-11-25 (#122 to #133): agent sessions in the terminal server, agent profiles and zones, Markdown, continuing an ended or lost agent, and logging in. The suite never sends real keys or pointer presses, and it never runs a real agent. So these need a person, a real pointer, IBus, and the free-tier agents on this machine.

Tick each box as you go. Note anything odd under the check, with the platform and theme.

## Setup

Build from `main` and run against a private state and server, so your daily Brindle and its saved session aren't touched. The run uses the config file you name in place, so edits to it apply live:

```sh
cargo build --release
scripts/run-throwaway.sh --config /tmp/agents-check.toml
```

On Windows:

```powershell
cargo build --release
scripts/run-throwaway.ps1 -Config C:/Temp/agents-check.toml
```

Use this config. It holds one profile per live agent. Codex and Claude come through their ACP adapters, and Claude only on Haiku, used sparingly.

```toml
[[profiles]]
name = "Copilot"
type = "agent"
command = "copilot"
args = ["--acp"]

[[profiles]]
name = "Codex"
type = "agent"
command = "npx"
args = ["-y", "@agentclientprotocol/codex-acp@2.12.0"]

[[profiles]]
name = "Antigravity"
type = "agent"
command = "agy"
args = ["acp"]
```

- **Windows:** run the npx agents through `command = "cmd"` and `args = ["/c", "npx", …]`.
- **Antigravity's terminal-login checks on Windows:** the profile must run `agy` directly, because a `cmd /c` profile withholds that method.
- **Over SSH:** give a profile `remote = { type = "ssh", host = "<host>" }`.
- **Opening an agent:** use the strip's `⌄` menu, or `brindle agent NAME`.

## 1. Agent sessions in the terminal server (#122)

- [ ] **Upgrade with tabs open (Linux).** With tabs open in your daily Brindle, reinstall it with `scripts/install.sh` and start the new build. The tabs reattach with their scrollback: the holder moves from H1 2 to H1 3, and the new core adopts the old holder's sessions. `brindle-remote list` shows the sessions as before.
- [ ] **Windows zip.** Unpack the `brindle-windows-x64` release artifact and open a local tab. `brindle-remote.exe`'s Properties → Details still show the version and icon.

## 2. Agent profiles and zones (#123)

- [ ] **A real agent end to end, on Linux and on Windows** (through `cmd /c npx …`):
  - a turn, a tool call, and a permission;
  - a queued prompt, and a held one sent again;
  - cancel, and close with its confirmation;
  - quit and relaunch: the conversation restores, and the first prompt isn't sent again;
  - the sessions palette lists the agent with its status.
- [ ] **IME in the prompt input (IBus):**
  - ctrl-shift-u `e9` space gives `é`;
  - enter while composing commits the composition and doesn't send;
  - the candidate window sits at the cursor on a wrapped line.
- [ ] **Scrolling:**
  - wheel scrolling a conversation, both following the end and leaving it;
  - the scroll-to-bottom button;
  - a pointer press into an inactive agent zone focuses it.
- [ ] **Pointer presses in an agent zone:**
  - the permission buttons and the stop button;
  - the queue row's remove, send-again and discard buttons;
  - a thought's trigger;
  - a message's copy button, which shows on hover;
  - the close dialog's buttons and its backdrop;
  - in an agent profile opened from the `⌄` menu, each of the header's split buttons opens another agent zone beside it.
- [ ] **A desktop notification from an agent in a background tab** (D-Bus on Linux, a toast on Windows): for a permission, for `Done` and for a failure. Clicking it shows the agent tab.
- [ ] **The settings dialog with an agent profile:**
  - its `agent` mark and note read in a light and a dark theme;
  - presses on the disabled Edit and Duplicate do nothing;
  - a double-click on its row shows the note rather than an editor.
- [ ] **A profile theme unlike the chrome's:** only the card takes the profile's background and foreground, and the components keep the chrome's colours.
- [ ] **A large profile `font_size`:** the whole zone scales, paddings included, while tooltips stay at the window's size.
- [ ] **`brindle agent … -p -`:**
  - in an interactive Linux shell, it's refused at once, without waiting for typing;
  - on Windows, `Get-Content -Raw task.md | brindle agent … -p -` in PowerShell 7 and in 5.1 sends the file's text, both run alone and handed to a running Brindle;
  - `-p -` with nothing redirected, in PowerShell and in cmd, is refused with status 2.

## 3. Markdown in agent zones (#127)

Ask the agent for a reply with a code block, a table, a bare URL, `[the docs](https://…)` and a `mailto:` link.

- [ ] **Code block copy, with a real pointer, on Linux and on Windows:** a press on the copy button puts the block's code on the clipboard exactly as the agent wrote it, and focus stays where it was.
- [ ] **Links:**
  - a link's tooltip shows its target when its text differs, and a bare URL has no tooltip;
  - the pointing hand shows over a link only while ctrl is held;
  - ctrl-clicking an `https` link opens it, and a plain click does nothing.
- [ ] **The link confirmation:** ctrl-clicking a `mailto:` link opens it with the whole link shown and Cancel focused.
  - Enter by reflex, Cancel, and a press on the backdrop each close it, opening nothing.
  - Open runs the mail opener.
  - Focus returns to the prompt input every way.
- [ ] **A table:**
  - equal-width columns;
  - a bold header with a line under it, and lines between rows;
  - each column's alignment;
  - long cells wrap.
- [ ] **Legibility** in brindle-light, brindle-dark and solarized-dark: links, inline code, code blocks and their labels, and quotes.

## 4. Continuing an ended or lost agent (#132)

- [ ] **The Continue conversation button, with a real pointer, on Linux and on Windows:**
  - on an exited agent, a press starts the agent again, and focus returns to the prompt input;
  - the button looks right (outline, small, after the reason) in brindle-light, brindle-dark and solarized-dark, and at a large profile font size.
- [ ] **A real agent killed mid-conversation, then continued.** On Linux, kill it with `kill`. On Windows, run it through `cmd /c npx …` and kill it in Task Manager.
  - Codex continues by resume, and its earlier turns are kept.
  - Copilot continues by load, with the notice that its replay wasn't shown.
  - A prompt held at the kill is sent again once the agent is ready. Then run a new turn.
- [ ] **The holder under a real agent killed:** the zone says the agent was lost with its holder, and Continue conversation brings it back.

## 5. Logging in to an agent (#133)

To get a zone that needs a login, log the agent out first: Codex with `codex logout`, and Antigravity by signing out in `agy`. Then sign back in from the zone.

- [ ] **A real agent login from the zone, on Linux and on Windows:**
  - Codex's ChatGPT login, and Antigravity's Google OAuth login;
  - each at the start, and again after a session expired, when the held prompt is sent again afterwards;
  - Cancel login pressed once mid-login, after which the methods are offered again.
- [ ] **Antigravity's terminal login** (`terminal-login`, which runs `agy --login`), on Linux and over SSH:
  - the login zone opens beside the agent zone, and the login completes there;
  - the zone closes, and the agent starts again, ready.
- [ ] **Antigravity's terminal login through `cmd /c`, on Windows:** a profile that runs it that way shows the method withheld.
- [ ] **The login row, with a real pointer, on Linux and on Windows:**
  - a press on an agent method's button logs in with it;
  - a press on a terminal method's button opens its zone beside the agent zone, and that zone takes focus;
  - a press on a withheld method's muted button does nothing;
  - a press on Cancel login cancels, and also closes a terminal login's zone;
  - each method's description reads in brindle-light, brindle-dark and solarized-dark;
  - a `-p` first prompt waiting for a login shows in the input, which takes no typing.

## Not yet testable

- **Pointer presses in tests.** These checks could be automated with an in-process pointer hook. That's blocked by gpui 0.2.2 and recorded as the proposal-only change `debug-pointer-hook`.
- **The newer designs.** They are designs only, not built yet: tool content, commands, prompt context and images, and plans.
