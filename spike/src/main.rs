//! Throwaway spike for the `terminal-server` change, task group 0.
//!
//! Subcommands, each a self-contained experiment:
//!   exports   0.2  which pseudoconsole functions the inbox vs shipped ConPTY expose
//!   handover  0.2  can a second process take data/exit/HPCON over, exports only
//!   reparent  0.1  does a console dialog end up owned by a given HWND
//!   survive   0.3  does a detached/breakaway helper outlive its parent, incl. in a job
//!   pipe      0.4  per-user named pipe DACL + peer SID check + one IOCP over many sources
//!
//! Loads the shipped conpty.dll by absolute path, like src/terminal/conpty.rs.

use std::env;

mod conpty;
mod util;

mod exports;
mod handover;
mod pcon;
mod pipe;
mod reparent;
mod survive;

fn main() {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let rest = &args[2.min(args.len())..];
    match cmd {
        "exports" => exports::run(),
        "handover" => handover::run_origin(),
        "handover-taker" => handover::run_taker(rest),
        "survive" => survive::run_parent(rest),
        "survive-helper" => survive::run_helper(rest),
        "pipe" => pipe::run(),
        "pipe-client" => pipe::run_client(rest),
        "reparent" => reparent::run(),
        "dialog" => reparent::run_dialog(),
        other => {
            eprintln!("unknown subcommand {other:?}");
            eprintln!("usage: spike <exports|handover|reparent|survive|pipe> [args]");
            let _ = rest;
            std::process::exit(2);
        }
    }
}
