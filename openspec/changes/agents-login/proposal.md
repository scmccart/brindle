# Proposal

**Depends on:** `agents-profiles` (the agent zone and its login message, and through it `agents-sessions`' login-required agent state and client protocol 4), implemented and archived; and `agents-session-recovery`, whose `AGENT_RESTART` a terminal login reuses. It lands after recovery.
**Next:** `agents-log-paging`.

**Status: designed.** design.md, the delta specs and tasks.md are written (2026-11-25). Task 1.1, a capture spike, decides whether terminal logins are in this change; it found one (Antigravity's `terminal-login`, offered only when `auth.terminal` is advertised), so they are (design.md decision 1).

## Why

In step 1, when an ACP agent says it needs a login, the agent zone shows the agent's message and tells the user to log in through the agent's own command line, then open the profile again. The zone is a dead end: its prompt input is unavailable, a `brindle agent … -p` prompt waits as a draft, and the user has to find the agent's login command, run it in a terminal, and start over in a new zone. ACP has a way for the client to drive the login, and Brindle, being a terminal, is well placed to run one.

## What Changes

- When an agent needs a login, the agent zone offers the login methods the agent advertised in `initialize` (`authMethods`) and runs the one the user chooses, then carries on to `session/new`, so the zone becomes usable without being reopened.
- ACP v1 defines two kinds of method, and either or both could be offered:
  - **`agent`** (the default type): the client sends `authenticate { methodId }` on the ACP connection and the agent handles the login itself (often through a browser). In Brindle the terminal server is the ACP client, so the server would send it on the user's request.
  - **`terminal`**: the client launches the agent's own configured program interactively, with the method's `args` appended and its `env` applied, shows it to the user, and treats exit status 0 as success; it then reconnects to and reinitializes the agent. The client never sends `authenticate` for it, and must advertise `clientCapabilities.auth.terminal: true` in `initialize` before an agent may offer one.

Open questions, answered by the user on 2026-11-25 (design.md decision 1 says what each means):
- **Answers:** 1, `agent` always; `terminal` only if a capture spike (task 1.1) finds an agent offering one, else a proposal-only follow-up; Zed's `_meta.terminal-auth` ignored. 2, a terminal zone split beside the agent zone, on the profile's host; args and env shown before launch; deny-listed env keys dropped and named; withheld for commands that re-parse arguments (`cmd /c`, `.bat`/`.cmd`, `ssh`). 3, `agents-session-recovery`'s `AGENT_RESTART`, keeping the session and conversation. 4, a new `LoggingIn` agent state, the methods on `LoginRequired` and `AGENT_LOGIN`, all in client protocol 4 amended in place (unreleased, so not frozen). 5, one button per method in the agent's order, nothing preselected, no timeout, "Cancel login" sending `$/cancel_request`, closing always works. 6, the failed prompt is never resent; a held one gets send-again back once ready. 7, the `-p` first prompt is sent automatically once the login succeeds. 8, logout is out of scope.

The questions as asked:
1. **Which method types** Brindle supports: `agent`, `terminal`, or both.
2. **Where a `terminal` login runs:** a terminal zone beside the agent zone, a new tab, or the agent zone itself; and for a remote profile, on the endpoint. ACP says a client SHOULD NOT offer terminal login when the launch target re-parses arguments, such as a remote shell or a Windows batch file, which covers SSH profiles and the `cmd /c …` profiles step 1 asks for; it also says to show the appended `args` and `env` before launching, and not to apply `env` entries that change program lookup, loading or trust (`PATH`, `LD_*`, `NODE_OPTIONS`, proxies, …) without the user's approval.
3. **What "reconnect and reinitialize" means for an agent session** after a terminal login: restarting the agent process inside the same session (step 1 never restarts an agent), or a new session in the same zone.
4. **The client protocol:** asking the server to authenticate is a new client message, so a new client protocol version (`process R4`–`R6`); whether the advertised methods fit protocol 4's room for optional fields on an event or need new event forms.
5. **Choosing:** how the zone presents several methods, and that nothing is chosen for the user; whether a login can be cancelled, and whether `authenticate` waits with no timeout.
6. **Logins demanded later.** Step 1 handles a `-32000` from `session/new` and from a prompt, for example after the user logged out elsewhere, and both end in the same dead end: a prompt's `-32000` fails that turn, holds a prompt waiting in the slot with the reason "login required", and leaves the agent needing a login, with nothing sent again until the profile is opened anew (`agents-sessions`, "Login is left to the agent's own tools"). What is open is what happens after a login: whether the prompt that failed, or a held one, is offered again (ACP warns that resubmitting an inserted prompt can repeat tool side effects).
7. **The first prompt:** whether a `-p` prompt left as a draft by the login demand is sent after a successful login, or stays a draft.
8. **Logout:** whether the zone offers `logout` when the agent advertises `agentCapabilities.auth.logout`.

## What the full changes already say

- `agents-sessions`, design decision 4: `initialize` sends `clientCapabilities: {}`, with no `auth.terminal`, so in step 1 no agent may advertise a `terminal` method to Brindle.
- `agents-sessions`, design decision 17 and the `agent-sessions` requirement "Login is left to the agent's own tools": `-32000` from `session/new` gives `AgentState::LoginRequired { message }` with the agent's message; the server sends no `authenticate`, refuses prompts saying the login is done through the agent's own command line, keeps the agent running until the session is closed, and lists the session as `error`. This change replaces that requirement.
- `agents-sessions`, design decision 7: `LoginRequired { message }` carries only the message; the advertised methods aren't in the transcript. Decision 6 and the requirement "Transcript event forms are fixed for each protocol version": within protocol 4 an event may only gain optional fields; a new kind or value needs a new protocol version (question 4).
- `agents-profiles`, design decision 13: the zone says "<agent> needs you to log in. Log in through the agent's own command line, then open the profile again." with the agent's message; "`agents-login` brings it into the zone". The `agent-zones` requirement "An agent that needs a login" makes the prompt input unavailable and says Brindle SHALL NOT log in for the agent; this change modifies it.
- `agents-profiles`, `agent-zones` requirement "A launch's first prompt": when the agent needs a login, the `-p` prompt is left in the prompt input as an unsent draft (question 7).
- `agents-profiles`, design decision 8: a held prompt's send-again is unavailable while the agent is `LoginRequired`. Design decision 1 maps a login demand to status `error`.

ACP v2 renames `authenticate` to `auth/login` and `logout` to `auth/logout`, requires a `type` on every method and changes the terminal capability's shape. Speaking v2 is `agents-acp-v2`'s; this change needs only v1.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `agent-sessions` (added by `agents-sessions`): "Login is left to the agent's own tools" is removed; new requirements offer the methods, log in, cancel, and survive restarts; "Cancelling a turn", "Transcript event forms are fixed for each protocol version" and "The session list shows agent sessions" change; the terminal ones advertise `auth.terminal` (leaving "The server is the agent's ACP client" as it is) and restart an agent that needs a login.
- `agent-zones` (added by `agents-profiles`): "An agent that needs a login", "A launch's first prompt", "Agent zone status", "Keyboard path through an agent zone" and "Closing an agent mid-turn asks first" change; new requirements for choosing, running, cancelling and following a login, and for terminal logins.
- `terminal-server` is unchanged: protocol 4 is amended in place, so "Protocol versions" still reads true.

## Impact

- `remote-agent/src/acp/`: the reducer's start sequence and the `authenticate` request; possibly restarting an agent inside its session.
- `transcript/` (`brindle-transcript`) and `remote-protocol/`: the methods on the login-required state, a new client message, transcript tests and fuzzing for them.
- `src/agent/` and `ui/`: the login row in the zone, and a terminal for a `terminal` login if one is offered.
- The fake ACP agent gains login methods, for server tests and e2e cases.
