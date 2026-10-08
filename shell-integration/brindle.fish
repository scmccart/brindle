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
# out: fish measures its prompt's width itself and would count it. fish 4
# may send its own marks too; Brindle merges the duplicates.

if status is-interactive; and not set -q __brindle_si
    set -g __brindle_si 1

    function __brindle_preexec --on-event fish_preexec
        set -g __brindle_ran 1
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
