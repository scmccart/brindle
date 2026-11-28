#!/usr/bin/env python3
"""Logs an ACP agent's standard streams in the fixtures' format.

    acp-log-wrapper.py --log FILE -- AGENT [ARGS...]

Runs AGENT with its standard streams passed through line by line, and
appends each message to FILE as `c>a<TAB>JSON` (what Brindle sent) or
`a>c<TAB>JSON` (what the agent sent), as in remote-agent/tests/fixtures/acp.

With STRIP_DIRS=1 it deletes `params.additionalDirectories` from
`session/new`, `session/load` and `session/resume` before the agent reads
them (what is logged is what the agent received), to show an agent as it
behaved before Brindle named the attachment directory.

With SET_OPTIONS="id=value;id=value" it sets those session config options
itself (`session/set_config_option`) as soon as `session/new` answers, and
keeps the answers from Brindle. The requests and answers are logged, with
ids that start with `capture-`. That is how a run picks its permission
mode.

It is used as a profile's `command`, so it never writes to stdout itself.
"""

import json
import os
import subprocess
import sys
import threading

STRIPPED_METHODS = {"session/new", "session/load", "session/resume"}


def strip_dirs(line):
    """`line` without `params.additionalDirectories` of a session request."""
    try:
        message = json.loads(line)
    except ValueError:
        return line
    params = message.get("params") if isinstance(message, dict) else None
    if message.get("method") in STRIPPED_METHODS and isinstance(params, dict):
        if params.pop("additionalDirectories", None) is not None:
            return json.dumps(message, separators=(",", ":"))
    return line


def main():
    args = sys.argv[1:]
    if len(args) < 4 or args[0] != "--log" or args[2] != "--":
        sys.exit("usage: acp-log-wrapper.py --log FILE -- AGENT [ARGS...]")
    log_path, command = args[1], args[3:]
    strip = os.environ.get("STRIP_DIRS") == "1"
    options = [
        pair.split("=", 1)
        for pair in os.environ.get("SET_OPTIONS", "").split(";")
        if "=" in pair
    ]
    new_ids = set()
    injected = set()
    log = open(log_path, "ab", buffering=0)
    log_lock = threading.Lock()
    write_lock = threading.Lock()

    def record(direction, line):
        with log_lock:
            log.write(direction.encode() + b"\t" + line.rstrip(b"\r\n") + b"\n")

    child = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=None,
        bufsize=0,
    )

    def send(line):
        with write_lock:
            child.stdin.write(line.rstrip(b"\r\n") + b"\n")

    def to_agent():
        for line in iter(sys.stdin.buffer.readline, b""):
            if strip:
                line = strip_dirs(line.decode("utf-8")).encode("utf-8")
            record("c>a", line)
            if options:
                try:
                    message = json.loads(line)
                    if message.get("method") == "session/new":
                        new_ids.add(message.get("id"))
                except ValueError:
                    pass
            try:
                send(line)
            except OSError:
                break
        try:
            child.stdin.close()
        except OSError:
            pass

    threading.Thread(target=to_agent, daemon=True).start()
    out = sys.stdout.buffer
    for line in iter(child.stdout.readline, b""):
        record("a>c", line)
        if options:
            try:
                message = json.loads(line)
            except ValueError:
                message = {}
            if message.get("id") in injected:
                continue
            session = (message.get("result") or {}).get("sessionId")
            if session and message.get("id") in new_ids:
                new_ids.discard(message.get("id"))
                for number, (config, value) in enumerate(options):
                    request_id = "capture-%d" % number
                    injected.add(request_id)
                    request = json.dumps(
                        {
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "method": "session/set_config_option",
                            "params": {"sessionId": session, "configId": config, "value": value},
                        },
                        separators=(",", ":"),
                    ).encode()
                    record("c>a", request)
                    send(request)
        out.write(line)
        out.flush()
    sys.exit(child.wait())


if __name__ == "__main__":
    main()
