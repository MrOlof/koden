# koden-shell-integration (zprofile)
#
# See zshenv.zsh for the rationale on the trailing `:`.
{
  _koden_user_zdotdir="${KODEN_USER_ZDOTDIR:-$HOME}"
  [ -f "$_koden_user_zdotdir/.zprofile" ] && source "$_koden_user_zdotdir/.zprofile"
  unset _koden_user_zdotdir
}
:
