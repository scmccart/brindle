//! The fields of a `/proc/<pid>/stat` line the core reads for a session's
//! foreground program.
//!
//! Pure, so it is tested on every platform. The Linux `unix` reads the
//! file.

/// `/proc/<pid>/stat`'s fields after the command, counted from 0: state,
/// ppid, pgrp, session, tty_nr, tpgid (fields 3 to 8 in proc(5)).
pub const STAT_PGRP: usize = 2;
pub const STAT_TPGID: usize = 5;

/// The numeric `field` of a `stat` line, counted from the state after the
/// command, whose name may hold spaces and parentheses; `None` when the
/// line has no such field or it isn't a number.
pub fn parse_stat_field(stat: &str, field: usize) -> Option<i32> {
    let fields = stat.get(stat.rfind(')')? + 1..)?;
    fields.split_whitespace().nth(field)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_fields_count_from_after_the_command() {
        // A command with spaces and parentheses of its own.
        let stat = "4242 (my (odd) cmd) S 1 4240 4240 34816 4250 4194560 101 0 0 0";
        assert_eq!(parse_stat_field(stat, STAT_PGRP), Some(4240));
        assert_eq!(parse_stat_field(stat, STAT_TPGID), Some(4250));
        // One that holds `) (`, as if it were two fields.
        let stat = "17 (a) (b) S 1 17 17 0 -1 4194560";
        assert_eq!(parse_stat_field(stat, STAT_PGRP), Some(17));
        assert_eq!(parse_stat_field(stat, STAT_TPGID), Some(-1));
        // No controlling terminal.
        let stat = "7 (kthreadd) S 2 0 0 0 -1 2129984 0 0";
        assert_eq!(parse_stat_field(stat, STAT_TPGID), Some(-1));

        // A line without the field gives none: empty, without `)`, too
        // short, or not a number.
        assert_eq!(parse_stat_field("", STAT_PGRP), None);
        assert_eq!(parse_stat_field("garbage", STAT_TPGID), None);
        assert_eq!(parse_stat_field("42 (cut S 1 42 42 0 43", STAT_PGRP), None);
        assert_eq!(parse_stat_field("42 (sh) S 1 42", STAT_TPGID), None);
        assert_eq!(parse_stat_field("42 (sh)", STAT_PGRP), None);
        assert_eq!(parse_stat_field("42 (sh) S 1 x 42 0 43", STAT_PGRP), None);
    }
}
