//! 0.2 (part): which pseudoconsole functions each ConPTY exposes.
//!
//! The handover question turns on whether an HPCON can be rebuilt in another
//! process. This prints, for the shipped conpty.dll and for the inbox
//! kernel32, which of the pseudoconsole functions are present — in particular
//! `Pack` (build an HPCON from handles) and any inverse.

use crate::conpty;

pub fn run() {
    let shipped = conpty::load_shipped();
    let inbox = conpty::kernel32();

    println!("shipped conpty.dll ({}):", conpty::VERSION);
    for (label, name) in conpty::SHIPPED_NAMES {
        let present = conpty::addr(shipped, *name).is_some();
        println!("  {:<13} {}", label, if present { "present" } else { "ABSENT" });
    }

    println!("\ninbox kernel32.dll:");
    for (label, name) in conpty::INBOX_NAMES {
        let present = conpty::addr(inbox, *name).is_some();
        println!("  {:<13} {}", label, if present { "present" } else { "ABSENT" });
    }

    // Is there any export that extracts handles from an existing HPCON?
    // Probe the plausible inverse names on the shipped DLL.
    println!("\nprobing for an HPCON -> handles inverse on shipped conpty.dll:");
    for name in [
        windows::core::s!("ConptyUnpackPseudoConsole"),
        windows::core::s!("ConptyGetPseudoConsoleHandles"),
        windows::core::s!("ConptyExportPseudoConsole"),
    ] {
        let present = conpty::addr(shipped, name).is_some();
        // name bytes for display
        let s = unsafe { std::ffi::CStr::from_ptr(name.0 as *const i8) }.to_string_lossy();
        println!("  {:<34} {}", s, if present { "present" } else { "ABSENT" });
    }
}
