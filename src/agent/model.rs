//! The chat model of an agent zone (design.md decision 8): what the zone
//! shows and decides, on the UI thread, from the transcript the reader
//! thread folds ([`crate::agent::shared`]) and the session's life events.
//! GPUI-free: values in, values out (`fcis R1`, `R5`).

/// The client protocol version that carries agent sessions.
pub const AGENT_PROTOCOL: u32 = 4;

/// Why an agent session never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartFailure {
    /// No terminal server could be reached or started: the cause.
    NoServer(String),
    /// The endpoint's terminal server speaks client protocols below 4.
    TooOld { endpoint: String },
    /// The server refused the spawn: the command and the server's error
    /// verbatim (on Windows a batch file's hint included).
    Refused { command: String, cause: String },
}

/// Whether a connection that agreed on client `protocol` can start an
/// agent session on `endpoint`: only protocol 4 and up carry
/// `SPAWN_AGENT`, so an older server is refused before anything is sent.
pub fn agent_start(protocol: u32, endpoint: &str) -> Result<(), StartFailure> {
    if protocol >= AGENT_PROTOCOL {
        Ok(())
    } else {
        Err(StartFailure::TooOld { endpoint: endpoint.to_string() })
    }
}

/// The words for a server too old for agent sessions.
pub fn too_old_text(endpoint: &str) -> String {
    format!("the terminal server on {endpoint} is too old for agent sessions; it will update once its terminals have ended")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_sessions_need_client_protocol_4() {
        assert_eq!(agent_start(4, "this machine"), Ok(()));
        assert_eq!(agent_start(5, "this machine"), Ok(()));
        for old in [1, 2, 3] {
            assert_eq!(agent_start(old, "A8"), Err(StartFailure::TooOld { endpoint: "A8".into() }), "protocol {old}");
        }
        assert_eq!(
            too_old_text("A8"),
            "the terminal server on A8 is too old for agent sessions; it will update once its terminals have ended"
        );
        let refused = StartFailure::Refused { command: "fake".into(), cause: "not found".into() };
        assert_ne!(refused, StartFailure::NoServer("not found".into()));
    }
}
