# koden-shell-integration (bashrc)
#
# Differences vs zsh integration:
# - We emulate login-shell init manually (/etc/profile, profile files) because
#   bash ignores --rcfile when started with -l.
# - Pre-exec marker uses PS0 (bash 4.4+). On older bash (macOS default 3.2) we
#   skip it — a fragile DEBUG-trap alternative would clobber the user's own
#   traps and interact badly with debuggers.

if [ -z "$__KODEN_HOOKS_LOADED" ]; then
  __KODEN_HOOKS_LOADED=1

  [ -f /etc/profile ] && source /etc/profile
  [ -f /etc/bashrc ] && source /etc/bashrc
  if [ -f "$HOME/.bash_profile" ]; then
    source "$HOME/.bash_profile"
  elif [ -f "$HOME/.bash_login" ]; then
    source "$HOME/.bash_login"
  elif [ -f "$HOME/.profile" ]; then
    source "$HOME/.profile"
  fi
  # .bashrc may have been sourced already by .bash_profile; sourcing again is
  # safe for idempotent rc files (the common case). If yours has side effects
  # on reload, guard with a flag.
  [ -f "$HOME/.bashrc" ] && source "$HOME/.bashrc"

  _koden_urlencode() {
    local LC_ALL=C s="$1" i c
    for (( i=0; i<${#s}; i++ )); do
      c="${s:i:1}"
      case "$c" in
        [a-zA-Z0-9/._~-]) printf '%s' "$c" ;;
        *) printf '%%%02X' "'$c" ;;
      esac
    done
  }

  # Inside tmux (Koden ssh Spaces) an OSC never reaches the client unless it
  # rides a DCS passthrough: ESC P tmux; <sequence, every ESC doubled> ESC \.
  # Koden turns allow-passthrough on when it creates the session.
  _koden_osc() {
    if [ -n "$TMUX" ]; then
      local s="$1"
      s="${s//$'\e'/$'\e\e'}"
      printf '\ePtmux;%s\e\\' "$s"
    else
      printf '%s' "$1"
    fi
  }
  # Prompt markers are PS1/PS0 escapes, expanded by bash at prompt time, so
  # the tmux form is spelled in that syntax too (\e -> ESC, \\ -> backslash).
  if [ -n "$TMUX" ]; then
    _koden_ps_b='\[\ePtmux;\e\e]133;B\e\e\\\e\\\]'
    _koden_ps_c='\[\ePtmux;\e\e]133;C\e\e\\\e\\\]'
  else
    _koden_ps_b='\[\e]133;B\e\\\]'
    _koden_ps_c='\[\e]133;C\e\\\]'
  fi

  _koden_precmd() {
    local _koden_ret=$?
    _koden_osc "$(printf '\e]133;D;%s\e\\' "$_koden_ret")"
    _koden_osc "$(printf '\e]7;file://%s%s\e\\' "${HOSTNAME:-$(uname -n 2>/dev/null)}" "$(_koden_urlencode "$PWD")")"
    if [ -n "$KODEN_BLOCKS" ]; then
      # Host renders its own input bar: suppress the shell prompt (B marker
      # only) and reserve header/gap rows, mirroring the zsh integration.
      if [ -n "$_koden_block_seen" ]; then
        PS1='\n\n'"$_koden_ps_b"
      else
        PS1='\n'"$_koden_ps_b"
      fi
    elif [ -z "$__KODEN_PS1_INJECTED" ]; then
      PS1="$_koden_ps_b$PS1"
      __KODEN_PS1_INJECTED=1
    fi
    _koden_osc "$(printf '\e]133;A\e\\')"
  }

  case ":${PROMPT_COMMAND:-}:" in
    *":_koden_precmd:"*) ;;
    *) PROMPT_COMMAND="_koden_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}" ;;
  esac

  # Pre-exec marker via PS0 (bash 4.4+). PS0 is expanded just before a command
  # runs — cleaner than a DEBUG trap, which would clobber user traps and fire
  # on every command including inside PROMPT_COMMAND.
  if [ "${BASH_VERSINFO[0]:-0}" -gt 4 ] \
     || { [ "${BASH_VERSINFO[0]:-0}" -eq 4 ] && [ "${BASH_VERSINFO[1]:-0}" -ge 4 ]; }; then
    if [ -n "$KODEN_BLOCKS" ]; then
      # PS0 only expands, never executes: the arithmetic inside the array
      # subscript sets the seen flag while the unset array expands to nothing.
      PS0="$_koden_ps_c"'${_koden_noop[$((_koden_block_seen=1))]}'"${PS0:-}"
    else
      PS0="$_koden_ps_c${PS0:-}"
    fi
  fi

  _koden_precmd
fi
:

# `koden` CLI (modules/cli): defined only where Koden planted KODEN_EXE, so a
# copy of this file on a remote host stays inert.
if [ -n "$KODEN_EXE" ]; then
  koden() { "$KODEN_EXE" cli "$@"; }
fi
