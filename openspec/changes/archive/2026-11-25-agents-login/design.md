# Design

## Context

See proposal.md ("Why", "What the full changes already say"). As built by `agents-sessions` and `agents-profiles`:

- **The reducer** (`remote-agent/src/acp/reduce.rs`) turns `-32000` from `session/new` (`created`) or `session/prompt` (`prompted`) into `Phase::LoginRequired` and `AgentState::LoginRequired { message }`. From then on `readiness()` refuses prompts and resends with `Refused::LoginRequired`, and nothing more is sent. `initialize`'s answer is read by `wire::initialize_result`, which keeps only the protocol version and `agentInfo`: `authMethods` is dropped. `rpc.rs` writes `initialize` with `clientCapabilities: {}`.
- **The transcript** (`brindle-transcript`) has `AgentState::{Starting, Ready, LoginRequired { message }, Failed, Exited}` and `HoldReason::LoginRequired`, with fixtures under `transcript/tests/fixtures/v4/` (`agent-login-required.json`, `queue-held-login-required.json`).
- **Client protocol 4** (`remote-protocol/src/client.rs`) has `SPAWN_AGENT` (15) and `AGENT_PROMPT`…`AGENT_RESEND` (16–20). `AGENT_CANCEL` has no reply.
- **The zone** (`src/agent/model.rs`): `failure_text` shows "<agent> needs you to log in. Log in through the agent's own command line, then open the profile again." with the agent's message. `prompt_status` is `Disabled(LoginRequired)`. `resend_of` makes send-again available only on `Live(Ready)` with the session open. `settle_first` turns a `-p` first prompt into a draft on `LoginRequired`. Permission rows are `brindle-ui`'s `Confirmation` (`src/agent/view.rs`).
- **What real agents advertise** (`remote-agent/tests/fixtures/acp/`, all with `clientCapabilities: {}`): Codex `api-key` and `chat-gpt`; Antigravity `oauth-personal`, `oauth-business`, `gemini-api-key`, `agent-platform`; Copilot `copilot-login`, an `agent`-type method carrying Zed's `_meta.terminal-auth {command, args: ["login"]}`; the deprecated claude-code-acp `claude-login` ("Run `claude /login` in the terminal"); claude-agent-acp none. No method has a `type`, so every one is `agent`. Descriptions use Markdown backticks. With `auth.terminal` advertised, Antigravity adds a `terminal` method (task 1.1, decision 1).
- **The schema crate** (`=1.22.0`) has v1's `AuthMethod::{Agent, Terminal}` (an untagged `agent` fallback for anything that isn't `terminal`), `AuthenticateRequest { methodId }`, `AuthCapabilities { terminal: bool }` and `$/cancel_request` (`CancelRequestNotification { requestId }`), which the receiver may ignore but must still answer the request.

**Dependency.** This change lands after `agents-session-recovery`, which adds `AGENT_RESTART {req, session, command, args, env}` to client protocol 4: the client re-sends the command, arguments and environment; the core keeps the ended session's log, ACP session id, ACP version, cwd and capabilities; the user starts it with "Continue conversation". That design is being written in parallel. This change uses the message by that name and assumes only what is listed here (decision 8 names the one extension it needs).

## Goals / Non-Goals

**Goals:**
- Run an `agent`-type login from the zone, at start and when demanded by a prompt, cancellable, with no timeout.
- Carry on in the same session and zone after a login; never resend a failed prompt; send a `-p` first prompt once after a login.
- Terminal logins, if the capture spike (task 1.1) finds an agent that offers one.

**Non-Goals:**
- Logout (`agentCapabilities.auth.logout`): out of scope. Logout is account-wide, and ACP leaves live sessions' fate undefined.
- Zed's `_meta.terminal-auth` (Copilot's `copilot-login`): not ACP; such a method is offered as the `agent` method it is.
- An approval UI for environment variables a terminal login sets: denied keys are dropped and named (decision 9).
- Retrying a login-demanded start with no methods ("Try again" after logging in elsewhere): it would be a restart from `LoginRequired` (decision 8), but without a terminal method it isn't asked for. Opening the profile again stays the way.
- Markdown in method descriptions (decision 12).
- ACP v2's `auth/login` (decision 13).

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `AgentState::LoggingIn`, `LoginRequired.methods`, `LoginMethod`, `LoginKind`, the activity mapping | pure (event forms, fold) | `brindle-transcript`, `transcript/src/lib.rs` |
| `AGENT_LOGIN` (`AgentLogin`), the protocol 4 table and module doc | pure codec | `brindle-remote-protocol`, `client.rs` |
| Reading `authMethods` (`wire::auth_methods`), `authenticate` and `$/cancel_request` lines (`Rpc::authenticate`, `Rpc::cancel_request`), `Purpose::Authenticate` | pure | `brindle-remote-agent`, `src/acp/{wire,rpc}.rs` |
| `Phase::{LoginRequired(Login), LoggingIn}`, `Command::Login`, the new `Refused` values, `Moment::Login` | pure | `src/acp/reduce.rs` |
| The checkpoint's `methods` and the phases' fields | pure | `src/acp/checkpoint.rs` |
| Routing `AGENT_LOGIN` to the reducer; the restart from `LoginRequired` (if terminal logins are in) | shell | `src/server/core/agents.rs`, `clients.rs` |
| `respond-to`, `respond-error-to`; `args` sections and `record-env` (if terminal logins are in) | test binary (pure parser) | `src/fake_acp/{script,main}.rs` |
| `Session::login`, `Sent::Login`, the reply path | shell (threads) | `src/remote/local/session.rs`, `src/agent/mod.rs` |
| `LoginRow` and its rules, the first prompt's wait, `prompt_status`, `resend_of`, the close rule, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs`, `src/agent/close.rs` |
| What a terminal login runs, the deny list, the re-parse check (if terminal logins are in) | pure | `brindle`, `src/agent/login.rs` (new whole pure module) |
| The login row (method buttons, Cancel login), focus stops | shell (view) | `src/agent/view.rs` |
| The login zone beside the agent zone (if terminal logins are in) | shell | `src/zone_view.rs`, `src/agent/mod.rs` |

No crate reference changes (`ports R1`, `R3`). The schema crate stays named only in `remote-agent/src/acp/` (`only_the_acp_modules_name_the_schema_crate`). A new whole pure module (`src/agent/login.rs`) joins `fcis`'s table and `PURE_MODULES`.

## Decisions

### 1. The user's answers, and what they mean here

Recorded 2026-11-25 (proposal.md, "Open questions"):

1. **Method types:** `agent` methods always. Task 1.1 re-captures Copilot and claude-code-acp (Codex and Antigravity optionally) advertising `clientCapabilities.auth.terminal: true`. If any offers `type: "terminal"`, terminal methods are in this change; the tasks marked "(terminal)" and the requirements named in decision 14 are then done. Else they move to a proposal-only follow-up (`agents-login-terminal`) and `initialize` keeps `clientCapabilities: {}`. `_meta.terminal-auth` is ignored.
2. **Where a terminal login runs:** a terminal zone split beside the agent zone, on the profile's host, through the existing terminal spawn (decision 9).
3. **Reconnect and reinitialize:** recovery's `AGENT_RESTART`, keeping the session and the conversation (decision 8).
4. **Client protocol:** a new `AgentState::LoggingIn`, the methods on `LoginRequired`, and `AGENT_LOGIN`, all in protocol 4 amended in place (decision 2). The user overrode the optional-fields-only approach.
5. **Choosing:** one button per method in the agent's order, on the `Confirmation` row, name and description, nothing preselected; no timeout; "Cancel login" sends `$/cancel_request` and returns to the choice; closing always works (decisions 6, 7).
6. **Later demands:** the failed prompt is never resent; a held `LoginRequired` prompt gets send-again back once `Ready` (decision 5).
7. **The `-p` first prompt** is sent automatically once a login succeeds and the agent is ready (decision 10). The user overrode keeping it a draft.
8. **Logout:** out of scope.

**Capture spike (task 1.1, 2026-11-25): Antigravity offers a terminal method, so terminal logins are in this change.** Each agent was sent Brindle's `initialize` changed only by `"clientCapabilities":{"auth":{"terminal":true}}`, then `session/new`, with no prompt and no `authenticate`, and with `DISPLAY` and `WAYLAND_DISPLAY` unset and `xdg-open`/`BROWSER` stubbed; nothing tried to open a browser or a window. The captures are `remote-agent/tests/fixtures/acp/<agent>-<version>-auth-terminal.log`, scrubbed as `agents-sessions`' were (archived design decision 5). Each agent's `authMethods`:

- **Copilot 1.0.115** (`copilot --acp`): unchanged. One method, `copilot-login` ("Log in with Copilot CLI", "Run `copilot login` in the terminal"), with no `type`, so `agent`. Its `_meta.terminal-auth` (`{command: <the copilot binary's absolute path>, args: ["login"], label: "Copilot Login"}`) is the same as with `clientCapabilities: {}`: advertising `auth.terminal` doesn't turn it into a `terminal` method. `session/new` succeeded.
- **claude-code-acp 0.16.2**: unchanged. One method, `claude-login` ("Log in with Claude Code", "Run `claude /login` in the terminal"), no `type`. `session/new` succeeded. (Run from inside Claude Code it needs `CLAUDECODE` unset, or `session/new` fails `-32603`.)
- **Codex** (`@agentclientprotocol/codex-acp` 2.12.0): unchanged. `api-key` ("API Key", with `_meta.api-key.provider: "openai"`) and `chat-gpt` ("ChatGPT"), no `type`. `session/new` succeeded, after the `_auth/status_update` and `auth/status_update` notifications.
- **Antigravity 1.5.16** (`agy acp`): the four methods it sends with `clientCapabilities: {}` (`oauth-personal`, `oauth-business`, `gemini-api-key`, `agent-platform`, no `type`), then a fifth that appears only when `auth.terminal` is advertised (checked by a second run with `{}`): `{"id":"terminal-login","name":"Log in from the terminal","description":"Log in with your Google account by pasting a code in a terminal; works over SSH","type":"terminal","args":["--login"]}`, with no `env`. `agy acp --help` lists `--login` as "Log in with Google interactively, then exit (ACP terminal auth)", so decision 9's plan for an `agy acp` profile runs `agy acp --login`. `session/new` answered `-32000` "Authentication required": with no display its keyring collection couldn't be unlocked ("Silent sign-in failed" in its log), so the capture is also a real start-time login demand that offers a terminal method.

No agent sends `env` on a method, so the env handling (decision 9) is tested with the fake agent only. Task 1.2 keeps the "(terminal)" tasks and requirements, and `initialize` advertises `"clientCapabilities":{"auth":{"terminal":true}}` (task 3.2).

### 2. Client protocol 4 is amended in place (`process R4`–`R6`)

The agent-* changes ship together in one release, so the versions `agents-sessions` and `agents-profiles` introduced (client protocol 4, holder protocol H1 3) have not shipped and are not frozen. This change amends protocol 4 itself: a new enum value, new fields and a new client message, with no new version and **no fallback for a protocol 4 client built before this change**, since no such client is ever released. `process R5` ("a format that has shipped is frozen") holds: nothing here has shipped. No exception is needed in `docs/architecture/`. What changes in writing: `transcript/src/lib.rs`'s module doc and `.claude/rules/transcript.md` say protocol 4's forms freeze with the release that first ships it, and the `agent-sessions` requirement "Transcript event forms are fixed for each protocol version" says the same (delta spec). Holder protocol 3 is untouched: the holder still stores the checkpoint as opaque bytes (`process R1`).

The forms (`brindle-transcript`):

```rust
enum AgentState {
    Starting,
    Ready { name?, version? },
    LoginRequired {
        message: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        methods: Vec<LoginMethod>,
    },
    LoggingIn { method: String, name: String },   // "agent": "logging_in"
    Failed { message },
    Exited { code?, stderr },
}

struct LoginMethod {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(flatten)]
    kind: LoginKind,
}

#[serde(tag = "kind", rename_all = "snake_case")]
enum LoginKind {
    Agent,
    Terminal {
        #[serde(default, skip_serializing_if = "Vec::is_empty")] args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")] env: BTreeMap<String, String>,
    },
}
```

- On the wire a method is `{id, name, description?, kind, args?, env?}`, as the user asked; in Rust, `args` and `env` exist only on a `Terminal` method (`types R1`): an `agent` method with arguments can't be built.
- `LoginKind::Terminal` is in the form even if task 1.1 finds no terminal method, so a later terminal change needs no new protocol version once protocol 4 has shipped. Without it, the server never sends it.
- `LoginRequired` with no methods encodes exactly as today, so `agent-login-required.json` is unchanged; new fixtures are added (`agent-login-required-methods.json`, `agent-login-required-terminal.json`, `agent-logging-in.json`). `env` is a `BTreeMap`, so its keys are sorted (`process R6`).
- `LoggingIn` carries the method's name as well as its id, so a client attaching mid-login (a replay sends only the current state) can say what is running.
- `Activity` (the session list's and the zone's status): `LoggingIn` is `working`, like `Starting`; `LoginRequired` stays `error`.

The client message (`remote-protocol`): `AGENT_LOGIN {req, session, method}` → `OK` or `ERROR`, client → core, protocol 4, the client that has the session attached only, as the other agent messages. Its type number is the next free one after recovery's `AGENT_RESTART` (and `agents-log-paging`'s `AGENT_FETCH`, if that lands first); it is fixed when the three changes stack, and the table in `client.rs`'s module doc says it. `AGENT_CANCEL` keeps its form; its meaning grows: while the agent is logging in it cancels the login (decision 7). Transcript tests: `client-agent-login.hex` (a login accepted, then one refused for an unknown method), and `AgentLogin` in `fuzz.rs`. Existing protocol 4 fixtures stay as they are; should one have to change because protocol 4 is amended, that is allowed for protocol 4's own fixtures in this release only, and said in the commit.

### 3. Reading the methods (`wire.rs`; `railway R2`, `R5`; `types R6`)

`wire::auth_methods(result: &Value) -> Methods` reads `initialize`'s `authMethods` by hand, one element at a time, as permission options already are, so one bad method never loses the others:

- no `type`, or `"agent"` → `LoginKind::Agent`;
- `"terminal"` → `LoginKind::Terminal { args, env }`, kept only when the server advertised `auth.terminal` (an agent that sends one anyway breaks ACP's MUST; it is dropped);
- any other `type`, a missing or non-string `id` or `name`, or a malformed `args`/`env` → dropped and counted.

Bounds: at most 16 methods (`MAX_LOGIN_METHODS`), `id` and `name` at most 1 KiB, `description` at most 4 KiB, at most 32 `args` of at most 4 KiB, at most 32 `env` entries of at most 4 KiB each. Text is `clean`ed as every other transcript text. When anything was dropped or cut, the reducer adds one warning notice ("N login methods from the agent were left out"). `_meta` is never read, so Copilot's `terminal-auth` is ignored. The result is kept on `AgentSession` as `methods: Vec<LoginMethod>` (in the agent's order) and sent with every `LoginRequired`.

**As built (2026-11-25).** `auth_methods(result, terminal_advertised)` returns the methods, the count left out and the count of descriptions cut. Besides the bounds above, the kept methods together are at most 256 KiB encoded (`MAX_LOGIN_METHODS_LEN`), the rest left out and counted: 16 methods at the per-method bounds would make a `LoginRequired` event of several MiB, which `pack_transcript` refuses past a frame's 1 MiB. An `id` or `name` that is empty, an `id` with a control character, a non-string `description`, and `env` keys that are empty or hold `=` are left out too; a description past 4 KiB is cut at a character boundary, and an `id`, `name`, argument or `env` entry past its bound leaves the method out (a cut argument would run something else). The notice is "N login method(s) from the agent was/were left out", with ", and N login method(s)' description(s) was/were cut" when both. `wire::initialize_result` removes `authMethods` before the schema crate decodes the answer, so a malformed method can't cost the `initialize`.

### 4. The reducer (`fcis R1`; `types R1`, `R2`; `process R3`)

`Phase` changes:

```rust
struct Login {
    name: Option<String>, version: Option<String>,   // from initialize, for the later Ready
    message: String,                                  // the agent's last login message
    session: Option<String>,                          // the ACP session a prompt's demand left
}
enum Phase {
    …,
    LoginRequired(Login),
    /// `authenticate` is out (`Purpose::Authenticate` in `Rpc::outgoing`).
    LoggingIn { login: Login, method: String },
    …,
}
```

`Command::Login { method }` (from `AGENT_LOGIN`), and `Refused::{NoLoginNeeded, AlreadyLoggingIn, NoSuchMethod { method }, RunByClient { method }, LoggingIn}`, each with its text ("the agent isn't waiting for a login", "a login is already running", "the agent offers no login method \"x\"", "login method \"x\" runs in a terminal, which Brindle opens", "the agent is logging in"). `readiness()` maps `LoginRequired` to `Refused::LoginRequired` and `LoggingIn` to `Refused::LoggingIn`.

| Phase | Input | Output |
| --- | --- | --- |
| `Creating` | `session/new` → `-32000` | `LoginRequired(Login { session: None, message })`; `Agent { LoginRequired { message, methods } }`; `Moment::Started` |
| `Ready { session }` | prompt → `-32000` | as today (turn failed, waiting prompt held `LoginRequired`), then `LoginRequired(Login { session: Some(session) })` with the methods |
| `LoginRequired` | `Login { method }`, an `agent` method | `authenticate {methodId}` (`Purpose::Authenticate`); `LoggingIn`; `Agent { LoggingIn { method, name } }`; `Moment::BeforeRequest`; reply `Ok` |
| `LoginRequired` | `Login`, a `terminal` method / unknown id | reply `RunByClient` / `NoSuchMethod`, nothing else |
| `LoggingIn` | `authenticate` → result | `Agent { Starting }`; with `session: None`, `session/new` and `Creating { name, version }`; with `Some(s)`, `Ready { session: s }` and `Agent { Ready { name, version } }` in the same step; `Moment::Login` |
| `LoggingIn` | `authenticate` → error | `LoginRequired(Login { message: error })`; `Agent { LoginRequired { message, methods } }`; `Moment::Login` |
| `LoggingIn` | `Cancel` | `$/cancel_request {requestId}` written; the id forgotten (`Rpc::forget_id`); `LoginRequired(login)` with its earlier message; `Agent { LoginRequired … }`; `Moment::Login` |
| any | an answer to a forgotten id | ignored, as every unknown id is today |
| `LoginRequired`, `LoggingIn` | `Prompt`, `Resend` | refused; `Unqueue` works as today |
| `LoggingIn` | `Exited` | as today from any phase: `Exited`, the authenticate forgotten |

- The waiting prompt held at a prompt's demand stays held through the login; `resend_of` in the zone and the reducer's resend both key on `Ready`, so send-again returns with nothing new (decision 5).
- `session/new` after a login can answer `-32000` again; that is the `Creating` row.
- `-32000` from `authenticate` itself is just an error: back to `LoginRequired` with its message.
- `output_lost` while `LoggingIn` (a truncated tail lost the answer): forget `Purpose::Authenticate`, back to `LoginRequired` with "the login's answer was lost while the terminal server restarted". The user chooses again.
- `Activity` (decision 2) is computed from the transcript, so the list follows.

**As built (2026-11-25).** `Login` is `{ greeting: Greeting, message, then: AfterLogin }` with `AfterLogin::{Start(Start), Ready { session }}` rather than `{ name, version, message, session }`. Two things the table needs didn't fit the sketch: `Creating` and the continue phases carry a whole `Greeting` (its `continuation` goes into the later `Conversation`), and recovery's `session/resume` or `session/load` can answer `-32000` too, where `session: None` would have fallen back to `session/new` and dropped the conversation (recovery's "no continue falls back to `session/new`"). So a start's demand keeps its `Start` (`New`, or the continue rebuilt from the phase: the conversation id, the session's directory, the greeting's continuation, and the log, a muted load being `Kept` and a shown one `Fresh`) and sends it again after the login; a prompt's demand keeps its ACP session, its greeting rebuilt from the log's `Ready` and conversation. The checkpoint's `session` field still tells the two apart (decision 15); the login phases also carry the greeting's `continuation`, and a continue's demand its `start`, as a spawning phase does. Also: a successful `authenticate` answer is taken whatever its result's shape; `AGENT_CANCEL` while only `LoginRequired` does nothing; a login command in any phase other than `LoginRequired` and `LoggingIn` is `NoLoginNeeded`; `Refused::LoginRequired`'s text is now "the agent needs a login" (the server no longer sends a login through the agent's own command line); and `Exited` while logging in forgets the `authenticate`. The core's routing of `AGENT_LOGIN` to `Command::Login` (task 5.1's code, not its real-role tests) landed with the reducer, since the stopgap `ERROR` it replaced was only there to keep the build whole.

### 5. Prompts around a login

Nothing in the reducer sends a prompt on its own after a login: the failed turn stays `Failed`, its user entry stays in the log, and the server has no record of it as resendable (`agent-sessions`, "Prompts while a login is needed"). A held prompt is sent again only by `AGENT_RESEND`. In the zone, `resend_of` already returns `Available` on `Live(Ready)`; the only change is its text for `LoginRequired` ("the agent needs a login") and a new one for `LoggingIn` ("the agent is logging in").

### 6. The login row (`agent-zones`; `fcis R1`, `R5`)

`ChatModel` gains a `LoginRow` it computes from the phase (pure, unit-tested):

```rust
enum LoginRow {
    None,
    /// The agent's message, then one choice per method.
    Choose { message: SharedString, methods: Vec<MethodChoice> },
    Running { name: SharedString },
}
struct MethodChoice {
    id: SharedString, name: SharedString, description: Option<SharedString>,
    offer: Offer,
}
enum Offer {
    Agent,
    Terminal { shown: TerminalShown },          // (terminal) decision 9
    Withheld { why: SharedString },             // (terminal) decision 9
}
```

- `failure_text` for `LoginRequired`: with at least one choice that isn't withheld, "<agent> needs you to log in." with the agent's message as detail; with none, today's text ("… Log in through the agent's own command line, then open the profile again."). `DisabledWhy::LoginRequired`'s text follows the same split; `DisabledWhy::LoggingIn` is "the agent is logging in".
- The view draws the row with `brindle-ui`'s `Confirmation`, as permission rows: state `Requested`, title "Log in to continue", one `ConfirmationAction` per choice in order, keyed by the method id (`ItemKey`), labelled with the name, the description under it; all of one plain variant, none primary, nothing focused. A withheld choice is a disabled action with its reason. `Running` is the same row with the title "Logging in with <name>…" and one action, "Cancel login". No new `brindle-ui` component; if `Confirmation` lacks a per-action description, a small additive option is added to it (`ui-components` doesn't change behaviour for permissions).
- Focus stops join the zone's `FocusStops` after the queue row's and before permission rows': `login:<id>` per enabled method, or `login:cancel`. Nothing takes focus when the row appears (`agent-zones`, "Nothing is answered for the user" applies by analogy).
- A choice sends `AGENT_LOGIN` (`Session::login`, `Sent::Login`); an `ERROR` reply shows its text beside the row, as a refused prompt's does. A press while a request is in flight does nothing.

**As built (2026-11-25, tasks 6.1 and 7.2).** `Running` is `Running { method, name, terminal }` (`terminal` for a terminal login, decision 9), and `Offer` is named `MethodOffer` (`Agent`, `Terminal(TerminalLogin)`, `Withheld(Withheld)`, in `login.rs`), since recovery's continue offer is already `Offer`. The split text is two variants, `DisabledWhy::LoginRequired` ("the agent needs a login") and `LoginElsewhere` (today's command-line words), and `failure_text` takes whether any method is usable. A refused `AGENT_LOGIN` is `ChatModel::login_note`, shown under the row's title (and cleared by the next choice or once the agent needs no login). The row sits above the prompt input with the alert, not in the conversation list. `Confirmation` had no per-action description, so `ConfirmationAction` gained `description` (plain text under the button; any description stacks the actions) and `disabled` (muted, no stop, asking nothing), with a gallery check and a `ui-components` delta requirement. A method's description shows the agent's text, then a terminal login's `shown()` or a withheld one's reason, joined by " · ".

### 7. Cancelling, timeouts, closing

- **No timeout** anywhere: not in the reducer, the client or the row. A browser OAuth can take minutes.
- **Cancel login** sends `AGENT_CANCEL` (decision 2): no new message is needed, since no turn can run while logging in. The server writes `$/cancel_request` and returns to `LoginRequired` at once, without waiting for the agent's `-32800`, because ACP lets the agent ignore the notification. A late answer to the cancelled id is ignored, success included: the user cancelled, and choosing again is quick if the agent did log in. The `agent_cancel` action ("Stop Agent Turn") does not cancel a login; only the button does.
- **Closing** works as always, ending the agent. A logging-in agent's status is `working`, but closing doesn't ask: `close.rs`'s `Closing::Agent.busy` is computed from the model (`ChatModel::close_busy`), which is `None` while `LoggingIn`. Nothing a login holds is lost that choosing again won't redo.

### 8. Reconnect and reinitialize: recovery's restart (`process R2`, `R3`)

A terminal login ends with ACP's "reconnect and reinitialize": the agent process must be started anew. This change uses `agents-session-recovery`'s `AGENT_RESTART {req, session, command, args, env}`, with the profile's own command, arguments and environment (not the login method's). The one extension it needs: recovery accepts `AGENT_RESTART` for a session whose agent has ended; this change also accepts it in `Phase::LoginRequired`, where the agent still runs. The core then hangs the agent up (decision 2 of `agents-sessions`: stdin closed, then ended after the grace), and the agent's exit, while a restart is pending, doesn't end the session or record `Exited`: it starts the restart, with recovery's machinery from there. The start is `Start::New` (`session/new`) when `Login.session` is `None`, and recovery's resume or load of `Login.session` otherwise, so the conversation is kept either way, and `initialize` carries `auth.terminal`. If recovery's design names its restart-pending phase or start enum differently, this change follows its names; if it can't restart without an ACP session id, this change adds the `session/new` start.

This is only needed for terminal logins. An `agent` login keeps the process: ACP's `authenticate` is a request to the running agent.

**As built (2026-11-25, task 5.3).** The reducer takes `Input::Restart` in `Phase::LoginRequired` and moves to a new `Phase::Restarting { start }`, with `Output::hang_up`, the info notice "the agent is started again after its login" and `AgentState::Starting` (so the client sees the agent starting while the old one is hung up; nothing more is read from it, prompts are `NotReady`, a login is `NoLoginNeeded`). `start` is `Login.then`'s start for a start's demand; for a prompt's demand it is a kept log's continue of its ACP session (`Start::Continue { conversation, log: Log::Kept }`, in the session's directory, as recovery's restart makes), and the restart is refused `CannotContinue` when the agent's `initialize` offered neither `session/resume` nor `session/load`: as recovery, no continue falls back to `session/new`. Recovery's restart had no restart-pending state for a running agent, so the core keeps the `AGENT_RESTART` on its `Agent` (`login_restart`, in memory only) and, at the holder's `EXITED`, steps `Input::Restart` again (`Restarting` → `Spawning { start }`, resetting the RPC, lines and tool ids as recovery's restart does) and queues it as an ended agent's restart (`start_again`, `SpawnRequest::Restart`), with no `Exited` in the log and no `EXIT`. `policy::restart_refusal` gains `needs_login`: a running agent that needs a login is restartable. The kept restart is dropped by any later hang-up (`CLOSE`, `KILL`, `STOP`, a gone `kill` client), by an exit with no client attached, and while the core stops or steps down; the exit is then stepped as any exit (from `Restarting`, an ordinary end: kept for its conversation when it can be continued, else `EXIT`). `Restarting` is encoded as `{"phase": "restarting", start?}` (additive in format 1). Only the step that hangs the agent up can store it (a hung-up session isn't checkpointed), and a core that adopts it has no kept restart, so the agent's exit ends it as any; the client can then continue the conversation, if any, as recovery's.

### 9. Terminal logins (terminal; `agent-zones`; `fcis R1`; `ports R4`)

Only if task 1.1 finds a terminal method. All decisions are pure, in `src/agent/login.rs`:

- `terminal_login(profile: &AgentProfileSpawn, method: &LoginMethod) -> Result<TerminalLogin, Withheld>`. `TerminalLogin { command, args, env, dropped: Vec<String> }`: the profile's command, its args followed by the method's, the environment the agent spawn builds (`agent_spawn_request`'s) with the method's `env` laid over it minus denied keys, and the denied keys, sorted.
- **The deny list** (`DENIED_ENV`, exact names and patterns, compared case-insensitively on Windows): `PATH`, `PATHEXT`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `NODE_PATH`, `PYTHON*`, `RUBYOPT`, `RUBYLIB`, `PERL5OPT`, `PERL5LIB`, `*_PROXY` (any case), `SSL_CERT_*`, `NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE`, `CURL_CA_BUNDLE`, `GIT_SSH*`, `BASH_ENV`, `ENV`, `ZDOTDIR`, `COMSPEC`. A denied key is dropped and named ("not applied: PATH"); no approval UI (an approval is additive later).
- **Withheld** (`Withheld { why }`) when the command re-parses its arguments: the program's stem (parsed as text, not `Path`, per the Windows pitfall in `CLAUDE.md`) is `cmd` with `/c` or `/k`, or ends `.bat` or `.cmd`; or the stem is `ssh` on any platform. An SSH *host* profile is not withheld: Brindle's spawns pass argv through the endpoint's helper.
- **Shown before launch:** the method's button shows its args joined with spaces and each applied `KEY=value`, then "not applied: K1, K2".
- **Running it:** the agent zone asks its `ZoneView` for a login zone: a terminal zone split beside it (right, else down when too narrow, as `can_split` decides), started through `Terminal::spawn` on the tab's `Host` with the `TerminalLogin` as its command, in the agent session's directory, with close policy `kill`, marked as a login zone. A login zone is the one exception to "a tab starts zones of its own kind only" (`zones`), and it is never recorded in the session state or a detached layout: on quit its session is closed, not detached (`process R16`: done in the quit path). One login zone per agent zone at a time; while it runs the row is `Running` with "Cancel login" closing the zone.
- **Its exit** closes the zone as any terminal zone's exit does. The agent entity watches it: status 0 sends `AGENT_RESTART` (decision 8); anything else shows the methods again with "the login program exited with status N" beside the row. A login zone the user closes is a cancel.

**As built (2026-11-25, tasks 6.5 and 7.4).**
- `terminal_login(command: &AgentCommand, args, env)` takes the profile's program and arguments and whether the host's variable names ignore case (`AgentCommand`, set on the model by the `Agent` entity from the profile and `LOCAL_ENV_IGNORES_CASE` on a local host), not `AgentProfileSpawn`. `TerminalLogin.env` holds only the method's applied variables (`appended` the method's own arguments, for the button); the zone runs `Terminal::spawn` with a clone of the profile whose `env` is `login::overlay(profile env, applied)`, so the base is the terminal spawn's environment (the profile's over Brindle's, the terminal variables kept, since the login runs in a terminal), not `agent_spawn_request`'s. The button's words are `shown()`: "Runs with ARGS; sets K=V, …; not applied: K, …".
- The re-parse check (`reparses`) runs on every platform; a batch file, `cmd /c|/k` and `ssh` re-parse their arguments wherever the profile names them.
- A terminal login is the client's alone: the server stays `LoginRequired` while it runs, and the model's `terminal_login` makes the row `Running { terminal: true }` and the prompt `Disabled(LoggingIn)`. So a quit or a crash while it runs leaves the agent needing a login, and the zone, restored, shows the methods again. `choose_login` refuses a second login while one runs. `cancel_login` returns `CancelLogin::CloseZone`, and the zone tree closes the login zone.
- The exit status reaches the zone through a new `ChannelEvent::Exited(Option<i32>)` (the server's `EXIT` code, where the reader sent a bare alacritty `Exit` before) and `Terminal::exit_code`; no status (a signal) shows "the login program was ended by a signal". `ZoneTreeView` owns the zone (`Zone::login_for`, the agent zone's id): it subscribes to each agent zone's `LoginZone::{Open, Close}`, splits the agent zone (`login_axis`: right when the agent zone is wide enough, else down; the size check only, since the split plan's same-kind rule doesn't apply), focuses the new zone (the login asks for typing), and reports its exit (`LoginZoneEnd::Exited`) or its close (`Closed`) to the agent once. Closing the agent zone closes its login zone. The command override gives `Closing::Ends` with policy kill, and `recorded_leaves` leaves login zones out of `zone_records` and `session_keys`, so neither session restore nor a detached layout records them.
- Status 0 restarts through `Agent::restart` with the profile found as recovery's continue finds it; a profile gone since shows `NoOffer::ProfileGone`'s words.

### 10. The `-p` first prompt after a login (`agent-zones`, "A launch's first prompt")

The user decided the first prompt is sent automatically once a login succeeds. Exactly:

- Only the zone's own `-p` prompt (`ChatModel.first`), only once, never from a replay, as today.
- `settle_first` gains a case: on `Live(LoginRequired)` with at least one choice that isn't withheld, it returns `First::Show(text)`: the view puts the text into the prompt input, which stays read-only (`PromptStatus::Disabled`), and the model keeps `first` pending. With no such choice it returns `First::Draft(text)` and drops `first`, as today. `LoggingIn` and `Starting` keep it pending.
- On the first live `Ready` with the link up, it returns `First::SendIfUnchanged(text)`: the view sends it (`Sent::FirstPrompt`) only if the input still holds exactly `text`, else drops it. While the input is read-only the user can't edit or clear it, so in practice it is always sent; the check is there so that no later path that makes the input editable can send text the user changed. **Flag:** to not send it, the user closes the zone. An editable-but-unsendable input (a new `PromptInput` status) was rejected as more UI for a rare case.
- A failed or cancelled login keeps it pending. The agent failing or exiting, the session being lost or ending, makes it a plain draft (`First::Draft`), as today.
- **The dump** while it waits: `--- prompt status=disabled first="TEXT"` (TEXT escaped as the other dump strings), and `--- input text=TEXT …` as for any focused input.

**As built (2026-11-25, tasks 6.2 and 7.2).** `First::Show` reaches the view as `AgentUpdate::SetDraft` (the input is read-only while disabled) and is given once (`first_shown`); `First::SendIfUnchanged` is `AgentUpdate::SendFirstIfUnchanged`, which the view answers with `Agent::send_first` when the input's text is exactly the prompt. A terminal login's restart keeps it pending too (`Starting`). In the dump `first=` comes before `refusal=` on the prompt line.

### 11. The dump and debug hooks (`testing R2`)

- `--- agent zone=ID state=…` gains `logging-in`.
- While `LoginRequired` with methods, one line per method after the agent line: `--- login choice method="ID" kind=agent|terminal "NAME"[ description="…"]` and, for a terminal method, ` args="…" env="K=V,…"[ dropped="K,…"]` or ` withheld="WHY"`. While `LoggingIn`: `--- login running method="ID" "NAME"`. A refused `AGENT_LOGIN` adds ` refusal="…"` to the last `--- login` line.
- Focus stops: `agent stop=login:ID` and `agent stop=login:cancel`.
- `--wait-for` matches these lines as it matches the other agent lines (`wait_text`).
- `CLAUDE.md`'s dump description is updated in the same task as the code (task 6.4).

**As built (2026-11-25, task 6.3).** A withheld terminal method's line has `kind=terminal` and ` withheld="WHY"`; a terminal login running is `--- login running …` as an agent one. The last `--- login` line's ` refusal="…"` also carries a login program's exit ("the login program exited with status N"). The e2e harness reads the lines as agent lines (`AGENT_LINES` gains `login `, `Dump::agent_logins`).

### 12. Descriptions stay plain (`agents-markdown`)

ACP v1 says nothing about the format of a method's description; v2 says auth-method descriptions are Markdown. Real descriptions use backticks ("Run `copilot login` in the terminal"). This change shows them as plain text with their backticks, one paragraph, wrapped. When `agents-markdown` has landed, drawing them with its renderer is a one-line change in the view and needs nothing on the wire; it is left out here so this change doesn't depend on that one.

### 13. ACP v2 (`agents-acp-v2`)

`Command::Login { method }` and `Purpose::Authenticate` are version-neutral; only `Rpc::authenticate` writes the v1 method name. `agents-acp-v2` changes the encoder to `auth/login`, reads v2's required `type`, and advertises `capabilities.auth.terminal: {}` instead of `true`. `LoginKind` is Brindle's own, so v2 changes no event form. That change also reshapes recovery's requests, which this change's restart (decision 8) goes through.

### 14. The requirements that move with terminal logins

If task 1.1 finds no terminal method, task 1.2 moves these out of this change's delta specs into a proposal-only change `agents-login-terminal` (its proposal naming them, with the spike's captures as its context), and drops the "(terminal)" tasks:

- `agent-sessions`: "Terminal login methods", "An agent that needs a login can be restarted".
- `agent-zones`: "A terminal login runs beside the agent zone", "A terminal login zone is not kept", "What a terminal login shows and applies", "Terminal logins are withheld from commands that re-parse arguments".

Everything else (decision 2's `LoginKind::Terminal` form included) stays.

### 15. What a core restart, a Brindle crash and an upgrade do (`process R1`–`R3`, `R12`)

- **Checkpoint** (`acp/checkpoint.rs`, format 1, additive): the state object gains `methods` (omitted when empty) and the `login_required` and `logging_in` phases gain `name`, `version`, `message`, `session` and `method` fields. Moments: `Moment::Login` on every login state change, and `BeforeRequest` before `authenticate` goes out, so the request id reaches the holder before its bytes (decision 11 of `agents-sessions`). Format 1 isn't bumped: no core that reads format 1 has shipped (decision 2). A core of an earlier build of this series that meets `logging_in` adopts the session as unreadable (`Failed`, left running), as `adopt` does for any unknown state; no release ever has that pairing.
- **Core restart:** the new core adopts `LoggingIn` with the `authenticate` id in `Rpc::outgoing`, so the agent's answer is applied (`agent-sessions`, "A login survives core restarts and upgrades"). A cancel sent before the crash was checkpointed with the id forgotten. A lost tail while logging in is decision 4's `output_lost` row.
- **Brindle crash or quit:** the session is detached with its login state; a client attaching later is replayed `LoginRequired` with the methods, or `LoggingIn`, and can cancel. A terminal login's zone is gone (its `kill` policy ends the program when the client is gone), and the agent zone is restored needing a login.
- **Upgrade:** as a core restart; the older core checkpoints and steps down, and the newer one carries on. A holder is never replaced while it has sessions, and holder protocol 3 is unchanged.

### 16. Tests (`testing R1`, `R7`, `R12`, `R13`, `R16`, `R18`)

- **Unit:** `wire::auth_methods` (each captured `initialize` answer, unknown types, bounds, a terminal method with and without `auth.terminal`); `Rpc::authenticate`, `cancel_request`, `forget_id`; the reducer, one test per row of decision 4's table; the checkpoint round trip of each new phase; `LoginRow`, `failure_text`, `resend_of`, `settle_first`'s new cases, `close_busy`; `login.rs`'s deny list, re-parse check and plan (terminal); the fake agent's new steps' parser.
- **Transcript:** the new `v4` fixtures and `client-agent-login.hex`; fuzzing for the new forms and `AgentLogin`.
- **Real-process** (`remote-agent/tests/agents.rs`, `upgrade.rs`, with `brindle-fake-acp-agent`): login at start, a failed login, a cancelled one with a late answer, a prompt's demand with a held prompt sent again after the login, a core killed while logging in, an upgrade while logging in; (terminal) `auth.terminal` advertised, a terminal method refused by `AGENT_LOGIN`, the restart from `LoginRequired` keeping the log.
- **e2e** (`tests/e2e/cases/agent.rs`): `agent-login` and `agent-first-prompt-login` updated for the no-method case; new `agent-login-methods`, `agent-login-fails`, `agent-login-cancel`, `agent-login-prompt`, `agent-first-prompt-after-login`, `agent-login-close`; (terminal) `agent-login-terminal`, `agent-login-terminal-fails`, `agent-login-terminal-env`, `windows-agent-login-withheld`, `remote-agent-login-terminal`.
- **Manual** (`CLAUDE.md`'s list): a real Codex ChatGPT login and an Antigravity Google OAuth login from the zone, and Antigravity's terminal login (task 1.1 found Copilot's isn't one); pointer presses on the method buttons and Cancel login.

**As built (2026-11-25, tasks 8.1–8.3).** The Windows cases are `windows-agent-login` (its own case, gated by the agent itself, not by a Shell tab: it answers only the second `authenticate`, so Brindle's waits see `logging-in`, Cancel login, the methods again, the other method chosen, then the `-p` prompt sent once), `windows-agent-login-withheld` (a `cmd /c` profile: the terminal method's `withheld=` line, not a stop, no login zone, no `authenticate`), and two more than the task names: `windows-agent-login-terminal` and `windows-agent-login-terminal-fails`, the Linux `agent-login-terminal` and `agent-login-terminal-fails` run on Windows unchanged, which check a login program's exit status through the holder's ConPTY (0 starts the agent again, 1 shows the methods with the status). `agent-login-terminal-env` isn't run on Windows: the environment's `PATH` may be named `Path` there. `remote-agent-login-terminal` runs a login at the start rather than after a prompt's demand. The restarted agent takes a new conversation, the `-p` prompt is sent to it once, the login program's recorded environment in the container has `LOGIN_MODE` and not `PATH=/nowhere`, and after the quit root's server lists the agent's session alone. The CI filters already select the new cases: `e2e-archive`'s `remote-` and `agent-`, and `windows-e2e-archive`'s `windows-agent-`.

## Risks / Trade-offs

- **[An agent's `authenticate` never returns]** → No timeout by decision; Cancel login and closing always work, and the row says which method runs.
- **[An agent ignores `$/cancel_request` and logs in anyway]** → The late success is ignored; the user chooses again, which is quick for a logged-in agent. The agent may hold a browser tab open; that is the agent's.
- **[An `agent` method that only says "run X in a terminal"]** (Copilot's `copilot-login`, claude-code-acp's `claude-login`) → Brindle sends `authenticate`, and the agent's error is shown with the methods again. The spike (decision 1) showed they stay `agent` methods with `auth.terminal` advertised.
- **[API-key methods]** (Codex `api-key`, Antigravity `gemini-api-key`) → They succeed only if the key is already in the profile's environment; the agent's error says so otherwise. No key entry UI.
- **[Auto-sending the `-p` prompt minutes later surprises]** → It shows in the input meanwhile with the dump's `first=`; closing the zone is the opt-out (flagged, decision 10).
- **[Amending protocol 4 after `agents-sessions` merged]** → No release carries protocol 4 yet; the release that does carries all agent-* changes (decision 2). If a release is cut between them, protocol 4 freezes there and this change needs protocol 5 instead: re-plan before such a release.
- **[Recovery's design differs from what decision 8 assumes]** → Only terminal logins depend on it; decision 8 states the one extension and follows recovery's names.
- **[The deny list misses a variable]** → It is a fixed, tested list in one pure module; adding to it is a one-line change. The button shows every applied variable before launch.
- **[A login zone in an agent tab breaks kind assumptions]** (layouts, restore, splits) → It is never recorded or detached, and its tab's kind stays `Agent`.

## Migration Plan

None for users: no config key, state file or released wire form changes. Protocol 4 clients and cores of this release all carry the change. Rollback is reverting the change; a session started by a newer build and adopted by an older core of this series is adopted as unreadable (decision 15).
