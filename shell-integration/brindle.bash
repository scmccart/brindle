# Brindle shell integration for bash: OSC 133 semantic prompt marks.
#
# Load it at the end of ~/.bashrc, after anything that sets PROMPT_COMMAND
# or PS1:
#
#     eval "$(brindle --shell-integration bash)"
#
# or, from a checkout:
#
#     source /path/to/brindle/shell-integration/brindle.bash
#
# It marks the prompt (A), where the command line starts (B), where the
# command's output starts (C; needs bash 4.4+ for PS0) and the command's end
# with its exit status (D). Just before C it reports the command line with
# VS Code's OSC 633 E, read from history. Terminals that don't know these
# sequences ignore them.

if [[ $- == *i* && -z ${__brindle_si-} ]]; then
    __brindle_si=1
    __brindle_status=0
    __brindle_first=1
    __brindle_hist=

    # The number of the newest history entry, or nothing when there is none.
    # `fc -l -0` prints `NUM<TAB> <line>` whatever HISTTIMEFORMAT says (only
    # `history` shows times).
    __brindle_newest() {
        local entry
        entry=$(fc -l -0 2>/dev/null) || return 0
        entry=${entry%%[[:space:]]*}
        [[ $entry =~ ^[0-9]+$ ]] && printf '%s' "$entry"
    }

    # In PS0, before C: the command line as typed, as OSC 633 E, if the
    # command added a history entry. A command that HISTCONTROL, HISTIGNORE
    # or `set +o history` keeps out adds none (nor does a repeat erasedups
    # moves to the end, which keeps its number), so nothing is sent and the
    # terminal reads the line from between B and C. With cmdhist (the
    # default) a command typed over several lines is one entry joined by
    # `; `, and with lithist one joined by newlines; either is sent as it is.
    # PS0 runs this in a subshell, so it only reads.
    __brindle_line() {
        local entry newest
        entry=$(fc -l -0 2>/dev/null) || return 0
        newest=${entry%%[[:space:]]*}
        [[ $newest =~ ^[0-9]+$ ]] && (( newest > ${__brindle_hist:-0} )) || return 0
        local line=${entry#"$newest"$'\t'}
        line=${line# }
        [[ -n $line ]] || return 0
        # `\` as `\\`, then `;` and control characters as `\xHH`. Quoted
        # replacements, so patsub_replacement's `&` can't apply.
        local bs='\' bs2='\\' semi=';' semi_x='\x3b' nl=$'\n' nl_x='\x0a'
        line=${line//"$bs"/"$bs2"}
        line=${line//"$semi"/"$semi_x"}
        line=${line//"$nl"/"$nl_x"}
        if [[ $line == *[[:cntrl:]]* ]]; then
            local out= c i
            for (( i = 0; i < ${#line}; i++ )); do
                c=${line:i:1}
                if [[ $c == [[:cntrl:]] ]]; then
                    printf -v c '\\x%02x' "'$c"
                fi
                out+=$c
            done
            line=$out
        fi
        printf '\e]633;E;%s\a' "$line"
    }

    # First in PROMPT_COMMAND: remember the command's exit status and hand it
    # on unchanged to whatever runs next.
    __brindle_save() {
        __brindle_status=$?
        return "$__brindle_status"
    }

    # Last in PROMPT_COMMAND: end the previous command and mark the prompt.
    # bash shows no PS0 for an empty line, so no C is sent for it, and
    # Brindle ignores a D that has no C: an empty line never looks failed.
    __brindle_prompt() {
        local status=$?
        if [[ -z $__brindle_first ]]; then
            printf '\e]133;D;%s\a' "$__brindle_status"
        fi
        __brindle_first=
        __brindle_hist=$(__brindle_newest)
        # Checked on every prompt, because prompt tools may rebuild PS1.
        if [[ $PS1 != *'\e]133;A'* ]]; then
            PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
        fi
        if [[ ${PS0-} != *'\e]133;C'* ]]; then
            # The line needs promptvars (on by default) to run in PS0.
            if shopt -q promptvars; then
                PS0="${PS0-}"'$(__brindle_line)\e]133;C\a'
            else
                PS0="${PS0-}"'\e]133;C\a'
            fi
        fi
        return "$status"
    }

    if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == "declare -a"* ]]; then
        PROMPT_COMMAND=(__brindle_save "${PROMPT_COMMAND[@]}" __brindle_prompt)
    else
        __brindle_nl=$'\n'
        PROMPT_COMMAND="__brindle_save${__brindle_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__brindle_nl}__brindle_prompt"
        unset __brindle_nl
    fi
fi
