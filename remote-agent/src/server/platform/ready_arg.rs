//! The ready pipe's argument of a server role, on both platforms: the role
//! is started as `<role> --ready-fd N` on Linux and `<role> --ready-handle
//! N` on Windows, or with no argument to stay in the foreground.
//!
//! Pure, so both platforms' rules are tested on every platform; each
//! platform's `daemon` turns the number into its pipe.

/// How a role is given its ready pipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyArg {
    /// A file descriptor number (Linux).
    Fd,
    /// A handle number (Windows).
    Handle,
}

impl ReadyArg {
    /// The argument before the number.
    pub const fn flag(self) -> &'static str {
        match self {
            Self::Fd => "--ready-fd",
            Self::Handle => "--ready-handle",
        }
    }

    /// What stands for the number in a role's arguments until the starter
    /// knows it.
    pub const fn placeholder(self) -> &'static str {
        match self {
            Self::Fd => "{ready_fd}",
            Self::Handle => "{ready_handle}",
        }
    }
}

/// The arguments that start `role` detached: `<role> <flag> <placeholder>`.
pub fn role_args(arg: ReadyArg, role: &str) -> Vec<String> {
    vec![role.into(), arg.flag().into(), arg.placeholder().into()]
}

/// The ready pipe's number in `role`'s arguments, as written: `None` for
/// no arguments, the number's text after the flag, and else the usage text.
fn split<'a>(arg: ReadyArg, role: &str, args: &'a [String]) -> Result<Option<&'a str>, String> {
    match args {
        [] => Ok(None),
        [flag, value] if flag == arg.flag() => Ok(Some(value)),
        _ => Err(format!("usage: brindle-remote {role} [{} N]", arg.flag())),
    }
}

/// The ready pipe's file descriptor in `role`'s arguments (`--ready-fd N`,
/// `N >= 0`), or `None` for a role that stays in the foreground. Nothing
/// else is accepted.
pub fn parse_fd(role: &str, args: &[String]) -> Result<Option<i32>, String> {
    let Some(fd) = split(ReadyArg::Fd, role, args)? else {
        return Ok(None);
    };
    fd.parse::<i32>()
        .ok()
        .filter(|fd| *fd >= 0)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{role}: {} needs a file descriptor number, not {fd:?}",
                ReadyArg::Fd.flag()
            )
        })
}

/// The ready pipe's handle in `role`'s arguments (`--ready-handle N`,
/// `N != 0`), or `None` for a role that stays in the foreground. Nothing
/// else is accepted.
pub fn parse_handle(role: &str, args: &[String]) -> Result<Option<usize>, String> {
    let Some(handle) = split(ReadyArg::Handle, role, args)? else {
        return Ok(None);
    };
    handle
        .parse::<usize>()
        .ok()
        .filter(|handle| *handle != 0)
        .map(Some)
        .ok_or_else(|| {
            format!(
                "{role}: {} needs a handle number, not {handle:?}",
                ReadyArg::Handle.flag()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn a_descriptor_of_zero_or_more_is_taken() {
        assert_eq!(parse_fd("hold", &[]), Ok(None));
        assert_eq!(parse_fd("hold", &args(&["--ready-fd", "3"])), Ok(Some(3)));
        assert_eq!(parse_fd("hold", &args(&["--ready-fd", "0"])), Ok(Some(0)));
        for bad in ["-1", "x", "", "3x"] {
            assert_eq!(
                parse_fd("hold", &args(&["--ready-fd", bad])),
                Err(format!(
                    "hold: --ready-fd needs a file descriptor number, not {bad:?}"
                )),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_nonzero_handle_is_taken() {
        assert_eq!(parse_handle("hold", &[]), Ok(None));
        assert_eq!(
            parse_handle("hold", &args(&["--ready-handle", "1234"])),
            Ok(Some(1234))
        );
        for bad in ["0", "-4", "x", ""] {
            assert_eq!(
                parse_handle("hold", &args(&["--ready-handle", bad])),
                Err(format!(
                    "hold: --ready-handle needs a handle number, not {bad:?}"
                )),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn each_parser_refuses_the_other_flag() {
        assert_eq!(
            parse_fd("hold", &args(&["--ready-handle", "3"])),
            Err("usage: brindle-remote hold [--ready-fd N]".to_string())
        );
        assert_eq!(
            parse_handle("hold", &args(&["--ready-fd", "3"])),
            Err("usage: brindle-remote hold [--ready-handle N]".to_string())
        );
    }

    #[test]
    fn role_arguments_name_the_flag_and_its_placeholder() {
        assert_eq!(ReadyArg::Fd.flag(), "--ready-fd");
        assert_eq!(ReadyArg::Fd.placeholder(), "{ready_fd}");
        assert_eq!(ReadyArg::Handle.flag(), "--ready-handle");
        assert_eq!(ReadyArg::Handle.placeholder(), "{ready_handle}");
        assert_eq!(
            role_args(ReadyArg::Fd, "hold"),
            args(&["hold", "--ready-fd", "{ready_fd}"])
        );
        assert_eq!(
            role_args(ReadyArg::Handle, "serve"),
            args(&["serve", "--ready-handle", "{ready_handle}"])
        );
    }

    #[test]
    fn anything_else_is_the_usage_text() {
        for list in [
            &["--ready-fd"][..],
            &["--other", "3"],
            &["--ready-fd", "3", "4"],
            &["3"],
        ] {
            assert_eq!(
                parse_fd("serve", &args(list)),
                Err("usage: brindle-remote serve [--ready-fd N]".to_string()),
                "{list:?}"
            );
        }
        for list in [
            &["--ready-handle"][..],
            &["--other", "3"],
            &["--ready-handle", "3", "4"],
        ] {
            assert_eq!(
                parse_handle("serve", &args(list)),
                Err("usage: brindle-remote serve [--ready-handle N]".to_string()),
                "{list:?}"
            );
        }
    }
}
