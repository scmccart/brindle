# Brindle shell integration for fish: OSC 133 semantic prompt marks.
#
# Load it at the end of ~/.config/fish/config.fish:
#
#     brindle --shell-integration fish | source
#
# or, from a checkout:
#
#     source /path/to/brindle/shell-integration/brindle.fish
#
# It marks the prompt (A), where the command's output starts (C) and the
# command's end with its exit status (D). B (command line start) is left
# out: fish measures its prompt's width itself and would count it. So just
# before C it reports the command line as typed with VS Code's OSC 633 E,
# the only way the terminal learns it. fish 4 sends its own marks too, which
# Brindle merges with these; its own C carries the line (`cmdline_url`) and
# comes first, so there the E only repeats it.

if status is-interactive; and not set -q __brindle_si
    set -g __brindle_si 1

    # The line as OSC 633 E: `\` as `\\`, then `;` and control characters as
    # `\xHH`. It is escaped a line at a time and joined with `\x0a`, since a
    # command substitution would split it at its newlines.
    function __brindle_line
        test -n "$argv[1]"; or return 0
        set -l parts (string split -- \n $argv[1])
        set parts (string replace -a -- '\\' '\\\\' $parts)
        set parts (string replace -a -- ';' '\\x3b' $parts)
        set -l line (string join -- '\\x0a' $parts)
        if string match -qr -- '[[:cntrl:]]' $line
            set -l out
            for c in (string split -- '' $line)
                if string match -qr -- '^[[:cntrl:]]$' $c
                    # `%09` from the URL style, as `\x09`.
                    set c (string escape --style=url -- $c | string replace -a -- '%' '\\x')
                end
                set -a out $c
            end
            set line (string join -- '' $out)
        end
        printf '\e]633;E;%s\a' $line
    end

    function __brindle_preexec --on-event fish_preexec
        set -g __brindle_ran 1
        __brindle_line $argv[1]
        printf '\e]133;C\a'
    end

    function __brindle_postexec --on-event fish_postexec
        set -g __brindle_status $status
    end

    function __brindle_prompt_start --on-event fish_prompt
        if set -q __brindle_ran
            printf '\e]133;D;%s\a' $__brindle_status
            set -e __brindle_ran
        end
        printf '\e]133;A\a'
    end
end
