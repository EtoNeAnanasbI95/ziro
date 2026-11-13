_ziro_disconnect() {
  if [[ -n $_ziro_fd ]]; then zle -F $_ziro_fd 2>/dev/null; exec {_ziro_fd}>&-; fi
  _ziro_fd='' _ziro_inflight=0 _ziro_tail='' _ziro_out=''
  _ziro_clear
}
_ziro_send() {
  [[ -n $_ziro_fd ]] || return 1
  local field
  for field in "$@"; do [[ $field != *[$'\0\x1f\x1e']* ]] || return 1; done
  setopt localoptions nomultibyte
  local wire="${(pj:\x1f:)@}" written=0
  (( ${#_ziro_out}+${#wire} < 65536 )) || { _ziro_disconnect; return 1; }
  _ziro_out+="$wire"$'\0'
  if syswrite -o $_ziro_fd -c written "$_ziro_out" 2>/dev/null; then
    _ziro_out=${_ziro_out[$((written+1)),-1]}
  elif (( written > 0 )); then
    _ziro_out=${_ziro_out[$((written+1)),-1]}
  fi
  return 0
}
_ziro_connect() {
  [[ -n $_ziro_fd ]] && return
  zsocket "$_ziro_socket" 2>/dev/null || return 1
  local low_fd=$REPLY
  exec {_ziro_fd}>&$low_fd
  exec {low_fd}>&-
  "$_ziro_binary" socket-nonblock <&$_ziro_fd 2>/dev/null || { _ziro_disconnect; return 1; }
  _ziro_send HELLO v1 "$_ziro_session" $$ "${TTY:-}" "${TERM:-}" "$ZSH_VERSION"
}
_ziro_read() {
  [[ $2 == (hup|err|nval) ]] && { _ziro_disconnect; return; }
  local chunk record
  sysread -i $_ziro_fd -s 65536 -t 0 chunk 2>/dev/null || { [[ $? == 5 ]] && _ziro_disconnect; return; }
  _ziro_tail+=$chunk
  (( ${#_ziro_tail} <= 1048576 )) || { _ziro_disconnect; return; }
  while [[ $_ziro_tail == *$'\0'* ]]; do
    record=${_ziro_tail%%$'\0'*}; _ziro_tail=${_ziro_tail#*$'\0'}
    local -a f=("${(@ps:\x1f:)record}")
    case $f[1] in
      CFG) _ziro_arrows=$f[2] _ziro_enter_accepts=$f[3]; _ziro_bind_all "${f[@]:4}" ;;
      FRAME) (( _ziro_inflight > 0 )) && (( _ziro_inflight-- )); [[ $f[2] == $_ziro_gen && $_ziro_hidden != "$BUFFER" ]] && _ziro_frame "${f[@]}" ;;
      CLEAR) (( _ziro_inflight > 0 )) && (( _ziro_inflight-- )); [[ $f[2] == $_ziro_gen ]] && _ziro_clear ;;
      ERR) _ziro_clear; [[ $f[2] == worker_timeout ]] && _ziro_worker_stop ;;
    esac
  done
  zle -R
}
