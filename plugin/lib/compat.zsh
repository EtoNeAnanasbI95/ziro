_ziro_compat_accept() { _ziro_dispatch ghost; }
_ziro_compat_word() { _ziro_dispatch ghost_word; }
_ziro_compat() {
  if (( $+functions[_zsh_autosuggest_disable] )); then
    _zsh_autosuggest_disable
    zle -N autosuggest-accept _ziro_compat_accept
    zle -N autosuggest-execute _ziro_compat_accept
    zle -N autosuggest-partial-accept _ziro_compat_word
  fi
}
