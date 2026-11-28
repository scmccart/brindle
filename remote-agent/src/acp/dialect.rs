//! The ACP version a session speaks (design decision 2 of
//! `agents-tool-content`, `ports R14`): [`Dialect`] is chosen once, from
//! `initialize`'s answer, kept by the session and its checkpoint, and is the
//! only place its variants are matched. Each method is one exhaustive
//! `match self` calling the same-named item of that version's module, as a
//! platform's module set exports the same names (`ports R5`), so a new
//! version fails to compile until every method has its arm.

use brindle_remote_protocol::serde_json::Value;

use super::envelope::{BadParams, Id, RpcError};
use super::forms::{
    Continued, Failure, Initialized, Listing, Methods, NewSession, NotificationKind, Offered, OfferedOption, OptionKey,
    PermissionRequest, Request, RequestKind, SessionUpdate,
};
use super::v1;
use brindle_transcript::StopReason;

/// An ACP protocol version Brindle speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    V1,
}

/// A protocol version Brindle doesn't speak: the agent's answer named it,
/// or a checkpoint recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unsupported {
    pub version: u64,
}

/// The `protocolVersion` an `initialize` answer names, read from the raw
/// result, since it is a number in every version; `None` when it is
/// missing or not a whole number.
pub fn spoken(result: &Value) -> Option<u64> {
    result.get("protocolVersion").and_then(Value::as_u64)
}

impl Dialect {
    /// The version `initialize` asks for.
    pub fn newest() -> Self {
        Self::V1
    }

    /// The dialect for protocol version `version`.
    pub fn from_version(version: u64) -> Result<Self, Unsupported> {
        match version {
            1 => Ok(Self::V1),
            _ => Err(Unsupported { version }),
        }
    }

    /// The protocol version this dialect speaks, as `initialize` and the
    /// checkpoint name it.
    pub fn version(self) -> u16 {
        match self {
            Self::V1 => v1::encode::VERSION,
        }
    }

    pub fn request_kind(self, method: &str) -> RequestKind {
        match self {
            Self::V1 => v1::decode::request_kind(method),
        }
    }

    pub fn notification_kind(self, method: &str) -> NotificationKind {
        match self {
            Self::V1 => v1::decode::notification_kind(method),
        }
    }

    pub fn initialize_result(self, result: &Value) -> Result<Initialized, BadParams> {
        match self {
            Self::V1 => v1::decode::initialize_result(result),
        }
    }

    pub fn auth_methods(self, result: &Value, terminal_advertised: bool) -> Methods {
        match self {
            Self::V1 => v1::decode::auth_methods(result, terminal_advertised),
        }
    }

    /// What the agent's error answer means, by this version's codes.
    pub fn failure(self, error: &RpcError) -> Failure {
        match self {
            Self::V1 => v1::decode::failure(error),
        }
    }

    pub fn new_session_result(self, result: &Value) -> Result<NewSession, BadParams> {
        match self {
            Self::V1 => v1::decode::new_session_result(result),
        }
    }

    /// A continue's answer: the options it offers (OpenSpec change
    /// `agents-commands`, decision 12).
    pub fn continued_result(self, how: Continued, result: &Value) -> Result<Offered, BadParams> {
        match self {
            Self::V1 => v1::decode::continued_result(how, result),
        }
    }

    /// The answer to a set of the option `key` names: the whole list when
    /// the answer carries one, `None` when it is empty by design.
    pub fn set_option_result(
        self,
        key: &OptionKey,
        result: &Value,
    ) -> Result<Option<Offered>, BadParams> {
        match self {
            Self::V1 => v1::decode::set_option_result(key, result),
        }
    }

    /// The options after a list from `config_option_update` or a set's
    /// answer replaces `held`'s: the version's own options outside the
    /// config options (v1's modes and model list) stay until the new list
    /// covers them.
    pub fn merge_options(self, held: &[OfferedOption], update: Offered) -> Offered {
        match self {
            Self::V1 => v1::decode::merge_options(held, update),
        }
    }

    /// The method that continues a conversation as `how` says, for a
    /// failure's text.
    pub fn continue_method(self, how: Continued) -> &'static str {
        match self {
            Self::V1 => v1::encode::continue_method(how),
        }
    }

    pub fn list_result(self, result: &Value) -> Result<Listing, BadParams> {
        match self {
            Self::V1 => v1::decode::list_result(result),
        }
    }

    pub fn prompt_result(self, result: &Value) -> Result<StopReason, BadParams> {
        match self {
            Self::V1 => v1::decode::prompt_result(result),
        }
    }

    pub fn session_update(self, params: &Value) -> Result<SessionUpdate, BadParams> {
        match self {
            Self::V1 => v1::decode::session_update(params),
        }
    }

    pub fn permission_request(self, params: &Value) -> Result<PermissionRequest, BadParams> {
        match self {
            Self::V1 => v1::decode::permission_request(params),
        }
    }

    pub fn cancel_request(self, params: &Value) -> Result<Id, BadParams> {
        match self {
            Self::V1 => v1::decode::cancel_request(params),
        }
    }

    /// The line that writes `request`.
    pub fn encode(self, request: Request<'_>) -> Vec<u8> {
        match self {
            Self::V1 => v1::encode::encode(request),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_remote_protocol::serde_json::json;

    #[test]
    fn only_version_1_is_spoken() {
        assert_eq!(Dialect::from_version(1), Ok(Dialect::V1));
        for version in [0, 2, 65535, u64::MAX] {
            assert_eq!(Dialect::from_version(version), Err(Unsupported { version }));
        }
        assert_eq!(Dialect::newest(), Dialect::V1);
        assert_eq!(Dialect::newest().version(), 1);
    }

    #[test]
    fn the_spoken_version_is_a_whole_number_or_none() {
        assert_eq!(spoken(&json!({"protocolVersion": 2})), Some(2));
        assert_eq!(spoken(&json!({"protocolVersion": 70000})), Some(70000));
        assert_eq!(spoken(&json!({"protocolVersion": "1"})), None);
        assert_eq!(spoken(&json!({"protocolVersion": -1})), None);
        assert_eq!(spoken(&json!({})), None);
        assert_eq!(spoken(&json!(null)), None);
    }
}
