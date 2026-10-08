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
# with its exit status (D). Terminals that don't know OSC 133 ignore them.

if [[ $- == *i* && -z ${__brindle_si-} ]]; then
    __brindle_si=1
    __brindle_status=0
    __brindle_first=1

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
        # Checked on every prompt, because prompt tools may rebuild PS1.
        if [[ $PS1 != *'\e]133;A'* ]]; then
            PS1='\[\e]133;A\a\]'"$PS1"'\[\e]133;B\a\]'
        fi
        if [[ ${PS0-} != *'\e]133;C'* ]]; then
            PS0="${PS0-}"'\e]133;C\a'
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
