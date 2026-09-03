# koden-shell-integration (zshrc)
#
# Emits OSC 7 (cwd) + OSC 133 A/B/C/D (prompt-start / prompt-end / pre-exec /
# command-done-with-exit-code) so the host can detect command boundaries and
# track cwd without re-parsing the prompt. `status` is a read-only special in
# zsh, so we shadow $? into `_koden_ret`.

{
  _koden_user_zdotdir="${KODEN_USER_ZDOTDIR:-$HOME}"
  [ -f "$_koden_user_zdotdir/.zshrc" ] && source "$_koden_user_zdotdir/.zshrc"
  unset _koden_user_zdotdir
}

# Re-source guard within a single shell (e.g. user runs `source ~/.zshrc`).
# This is NOT exported, so each nested zsh installs its own hooks — desired,
# since every interactive shell needs its own prompt integration.
if [[ -z "$__KODEN_HOOKS_LOADED" ]]; then
  __KODEN_HOOKS_LOADED=1
  autoload -Uz add-zsh-hook 2>/dev/null

  # URL-encode $PWD byte-wise so multi-byte paths stay valid in the `file://`
  # URI emitted via OSC 7. `no_multibyte` forces ${s[i]} to index bytes (not
  # code points), and LC_ALL=C keeps the [a-zA-Z0-9...] class single-byte.
  _koden_urlencode() {
    emulate -L zsh
    setopt localoptions no_multibyte
    local LC_ALL=C s="$1" i byte
    for (( i=1; i<=${#s}; i++ )); do
      byte="${s[i]}"
      case "$byte" in
        [a-zA-Z0-9/._~-]) printf '%s' "$byte" ;;
        *) printf '%%%02X' "'$byte" ;;
      esac
    done
  }

  # Inside tmux (Koden ssh Spaces) an OSC never reaches the client unless it
  # rides a DCS passthrough: ESC P tmux; <sequence, every ESC doubled> ESC \.
  _koden_osc() {
    if [[ -n "$TMUX" ]]; then
      local s="$1"
      s="${s//$'\e'/$'\e\e'}"
      printf '\ePtmux;%s\e\\' "$s"
    else
      printf '%s' "$1"
    fi
  }
  if [[ -n "$TMUX" ]]; then
    _koden_ps_b=$'%{\ePtmux;\e\e]133;B\e\e\\\e\\%}'
  else
    _koden_ps_b=$'%{\e]133;B\e\\%}'
  fi

  _koden_precmd() {
    local _koden_ret=$?
    _koden_osc "$(printf '\e]133;D;%s\e\\' "$_koden_ret")"
    _koden_osc "$(printf '\e]7;file://%s%s\e\\' "${HOST}" "$(_koden_urlencode "$PWD")")"
    # In block mode the host renders its own input bar, so suppress the shell
    # prompt entirely (keep only the OSC 133 B marker) and add a leading blank
    # line so frozen command blocks get vertical breathing room.
    if [[ -n "$KODEN_BLOCKS" ]]; then
      # Spacing reserved for the host-drawn block header lives in the prompt
      # (the grid is WebGL, not CSS). Later prompts get two blank rows: the
      # upper one is the previous block's end gap (above its divider), the lower
      # one is this command's header row. The very first prompt has no block
      # above it, so it gets a single row (header only) to avoid a tall top gap.
      if [[ -n "$_koden_block_seen" ]]; then
        PS1=$'\n\n'"$_koden_ps_b"
      else
        PS1=$'\n'"$_koden_ps_b"
      fi
      RPROMPT=''
    elif [[ "$PS1" != *']133;B'* ]]; then
      # Re-inject prompt-end marker in case a framework rebuilt PS1 (p10k, starship).
      PS1="$_koden_ps_b$PS1"
    fi
    _koden_osc "$(printf '\e]133;A\e\\')"
  }

  _koden_preexec() {
    # Mark that a real command ran, so the next prompt switches from one blank
    # row (first prompt, no block above) to two (end gap + header row).
    [[ -n "$KODEN_BLOCKS" ]] && _koden_block_seen=1
    local cmd="${1//[[:cntrl:]]/ }"
    _koden_osc "$(printf '\e]133;C;%s\e\\' "${cmd[1,256]}")"
  }

  if (( $+functions[add-zsh-hook] )); then
    add-zsh-hook precmd _koden_precmd
    add-zsh-hook preexec _koden_preexec
  fi

  # Warp/iTerm2-style word-end navigation: zsh's default `forward-word` (M-f /
  # Option+Right) overshoots to the START of the next word; `emacs-forward-word`
  # stops at the END of the current word, which is what nearly every other shell
  # and GUI editor does. Only rebind when the binding is still the stock zsh
  # default — respects any explicit remap in the user's .zshrc.
  if (( $+widgets[emacs-forward-word] )) \
     && [[ "$(bindkey '\ef')" == '"^[f" forward-word' ]]; then
    bindkey '\ef' emacs-forward-word
  fi

  _koden_precmd
fi
:

# `koden` CLI (modules/cli): defined only where Koden planted KODEN_EXE, so a
# copy of this file on a remote host stays inert.
if [[ -n "$KODEN_EXE" ]]; then
  koden() { "$KODEN_EXE" cli "$@"; }
fi
