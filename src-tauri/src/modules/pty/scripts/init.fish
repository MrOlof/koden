# koden-shell-integration (fish)
# Emits OSC 7 (cwd) + OSC 133 A/B/C/D so the host tracks cwd and prompt
# boundaries without re-parsing the prompt. fish 4.0+ writes its own OSC 133
# A/B (the `mark-prompt` feature); Koden disables it at spawn via
# fish_features=no-mark-prompt so these markers aren't emitted twice.

# Installed into conf.d, which every fish session sources; only Koden-spawned
# shells (KODEN_TERMINAL=1) may get their prompt wrapped.
if not set -q KODEN_TERMINAL
    exit 0
end
if set -q __KODEN_HOOKS_LOADED
    exit 0
end
set -g __KODEN_HOOKS_LOADED 1

# Koden is a clean terminal; drop fish's default startup greeting. A user who
# sets their own in config.fish (sourced after this) keeps it.
function fish_greeting
end

set -g __KODEN_HOST (uname -n 2>/dev/null; or echo localhost)

# URL-encode a path keeping `/` intact so it stays valid inside file://.
function __koden_urlencode_path
    set -l parts (string split '/' -- $argv[1])
    set -l out
    for p in $parts
        if test -n "$p"
            set out $out (string escape --style=url -- $p)
        else
            set out $out ""
        end
    end
    string join '/' $out
end

function __koden_restore_status
    return $argv[1]
end

function __koden_capture_user_prompt
    if not functions -q fish_prompt
        return
    end
    if functions fish_prompt | string match -q '*__koden_user_prompt*'
        return
    end
    functions -e __koden_user_prompt 2>/dev/null
    functions -c fish_prompt __koden_user_prompt
end

# Wrapped so `fish -C __koden_install_prompt` can re-run it AFTER config.fish,
# where a framework prompt (starship etc.) would otherwise override fish_prompt
# and drop our markers.
# Inside tmux (Koden ssh Spaces) an OSC never reaches the client unless it
# rides a DCS passthrough: ESC P tmux; <sequence, every ESC doubled> ESC \.
function __koden_osc
    if set -q TMUX
        printf '\ePtmux;%s\e\\' (string replace -a -- \e \e\e -- "$argv[1]")
    else
        printf '%s' "$argv[1]"
    end
end

function __koden_install_prompt
    __koden_capture_user_prompt
    if set -q KODEN_BLOCKS
        function fish_right_prompt
        end
        function fish_greeting
        end
    end
    function fish_prompt
        set -l __koden_status $status
        __koden_osc (printf '\e]133;D;%d\e\\' $__koden_status)
        __koden_osc (printf '\e]7;file://%s%s\e\\' "$__KODEN_HOST" (__koden_urlencode_path "$PWD"))
        __koden_osc (printf '\e]133;A\e\\')
        # Block mode: host renders its own input bar, so suppress the shell prompt
        # (B marker only) and reserve header/gap rows, mirroring zsh.
        if set -q KODEN_BLOCKS
            if set -q __koden_block_seen
                printf '\n\n'
            else
                printf '\n'
            end
            __koden_osc (printf '\e]133;B\e\\')
            return
        end
        __koden_restore_status $__koden_status
        if functions -q __koden_user_prompt
            __koden_user_prompt
        else
            printf '%s > ' (prompt_pwd)
        end
        __koden_osc (printf '\e]133;B\e\\')
    end
end
__koden_install_prompt

function __koden_preexec --on-event fish_preexec
    set -g __koden_block_seen 1
    set -l cmd (string replace -ra '[\x00-\x1f\x7f]' ' ' -- "$argv")
    __koden_osc (printf '\e]133;C;%s\e\\' (string sub -l 256 -- "$cmd"))
end

# `koden` CLI (modules/cli): defined only where Koden planted KODEN_EXE, so a
# copy of this file on a remote host stays inert.
if set -q KODEN_EXE
    function koden
        "$KODEN_EXE" cli $argv
    end
end
