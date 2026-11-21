# Brindle shell integration for zsh: OSC 133 semantic prompt marks.
#
# Load it at the end of ~/.zshrc, after your prompt theme:
#
#     eval "$(brindle --shell-integration zsh)"
#
# or, from a checkout:
#
#     source /path/to/brindle/shell-integration/brindle.zsh
#
# It marks the prompt (A), where the command line starts (B), where the
# command's output starts (C) and the command's end with its exit status
# (D). Just before C it reports the command line as typed with VS Code's
# OSC 633 E, so a right prompt or an autosuggestion never reads as part of
# it. Terminals that don't know these sequences ignore them.

if [[ -o interactive && -z ${__brindle_si-} ]]; then
  typeset -g __brindle_si=1 __brindle_status=0 __brindle_ran=

  # First precmd hook: remember the command's exit status.
  __brindle_save() {
    __brindle_status=$?
  }

  # Last precmd hook: end the previous command and mark the prompt. The
  # marks live in PS1 (checked every time, since themes rebuild it) so A
  # lands after zsh's PROMPT_SP line fix-up.
  __brindle_precmd() {
    if [[ -n $__brindle_ran ]]; then
      printf '\e]133;D;%s\a' "$__brindle_status"
      __brindle_ran=
    fi
    [[ $PS1 == *$'\e]133;A'* ]] || PS1=$'%{\e]133;A\a%}'"$PS1"
    [[ $PS1 == *$'\e]133;B'* ]] || PS1="$PS1"$'%{\e]133;B\a%}'
  }

  # The line as OSC 633 E: `\` as `\\`, then `;` and control characters as
  # `\xHH` (a newline in a command typed over several lines too). zsh gives
  # preexec the line as typed, also when history drops it, and an empty one
  # only when history is off: then nothing is sent.
  __brindle_line() {
    emulate -L zsh
    [[ -n $1 ]] || return 0
    local bs='\' bs2='\\' semi=';' semi_x='\x3b' nl=$'\n' nl_x='\x0a'
    local line=${1//"$bs"/$bs2}
    line=${line//"$semi"/$semi_x}
    line=${line//"$nl"/$nl_x}
    if [[ $line == *[[:cntrl:]]* ]]; then
      local out= c
      for c in "${(@s::)line}"; do
        [[ $c == [[:cntrl:]] ]] && printf -v c '\\x%02x' "'$c"
        out+=$c
      done
      line=$out
    fi
    printf '\e]633;E;%s\a' "$line"
  }

  __brindle_preexec() {
    __brindle_ran=1
    __brindle_line "$1"
    printf '\e]133;C\a'
  }

  typeset -ga precmd_functions preexec_functions
  precmd_functions=(__brindle_save "${precmd_functions[@]}" __brindle_precmd)
  preexec_functions+=(__brindle_preexec)
fi
