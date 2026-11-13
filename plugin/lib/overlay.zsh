# Private, inherited socket to the PTY proxy. No terminal control codes are
# emitted from ZLE: only the parent proxy owns popup drawing/restoration.
_ziro_overlay_init() {
  [[ ${ZIRO_RENDER_FD:-} == <-> ]] || return 0
  local inherited=$ZIRO_RENDER_FD
  if exec {_ziro_ui_fd}>&$inherited 2>/dev/null; then
    exec {inherited}>&-
    _ziro_overlay_mode=1
  fi
  unset ZIRO_RENDER_FD
}
_ziro_ui_send() {
  (( _ziro_overlay_mode )) && [[ -n $_ziro_ui_fd ]] || return 0
  local field
  for field in "$@"; do [[ $field != *[$'\0\x1f\x1e']* ]] || return 1; done
  setopt localoptions nomultibyte
  local wire="${(pj:\x1f:)@}" written=0
  if (( ${#_ziro_ui_out}+${#wire} > 262144 )); then
    exec {_ziro_ui_fd}>&-
    _ziro_overlay_mode=0 _ziro_ui_fd='' _ziro_ui_out=''
    return 1
  fi
  _ziro_ui_out+="$wire"$'\0'
  if syswrite -o $_ziro_ui_fd -c written "$_ziro_ui_out" 2>/dev/null; then
    _ziro_ui_out=${_ziro_ui_out[$((written+1)),-1]}
  elif (( written > 0 )); then
    _ziro_ui_out=${_ziro_ui_out[$((written+1)),-1]}
  fi
  return 0
}
