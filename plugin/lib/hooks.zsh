_ziro_precmd() {
  local exit_code=$?
  if [[ -n $_ziro_hist_id ]]; then _ziro_send HIST_END "$_ziro_hist_id" $exit_code $(( int((EPOCHREALTIME-_ziro_hist_start)*1000) )); _ziro_hist_id=''; fi
  if [[ -z $_ziro_fd ]]; then
    if ! _ziro_connect; then
      [[ -x $_ziro_binary ]] && HISTFILE=${HISTFILE:-} "$_ziro_binary" daemon --detach --socket "$_ziro_socket" 2>/dev/null
      _ziro_connect
    fi
  fi
  if [[ -n $_ziro_fd ]]; then
    if (( !_ziro_bound )); then
      # Initialization may wait here, before ZLE owns the keyboard.
      zselect -t 20 -r $_ziro_fd 2>/dev/null && _ziro_read
      (( $#_ziro_cfg_keys )) && _ziro_bind_all "${_ziro_cfg_keys[@]}"
    fi
    (( _ziro_gen++ ))
    _ziro_send CTX $_ziro_gen 0 ${COLUMNS:-80} ${LINES:-24} main "$PWD" '' ''
    _ziro_ui_send CTX $_ziro_gen 0 ${COLUMNS:-80} ${LINES:-24} main "$PWD" '' ''
    _ziro_worker_start
    _ziro_zwc_refresh
  fi
  _ziro_previous_buffer='' _ziro_previous_cursor=-1 _ziro_hidden='' _ziro_navigated=0 _ziro_explicit=0 _ziro_history_nav=0
}
_ziro_redraw() {
  local begin=$EPOCHREALTIME
  [[ -n $_ziro_fd && $_ziro_bound == 1 ]] || return
  zle -F $_ziro_fd _ziro_read_event
  [[ -n $_ziro_pty_fd ]] && zle -F $_ziro_pty_fd _ziro_worker_drain
  if [[ $_ziro_previous_buffer == "$BUFFER" && $_ziro_previous_cursor == $CURSOR && $_ziro_worker_cwd == "$PWD" && $_ziro_previous_cols == $COLUMNS ]]; then _ziro_paint; return; fi
  if [[ -z $BUFFER || ${KEYMAP:-main} == vicmd || $_ziro_hidden == "$BUFFER" ]]; then
    _ziro_clear
    [[ $_ziro_hidden == "$BUFFER" ]] && return
  else
    _ziro_invalidate
  fi
  _ziro_hidden=''
  local flags=''
  if (( _ziro_history_nav )); then
    if [[ $_ziro_history_buffer == "$BUFFER" ]]; then flags+=h; else _ziro_history_nav=0; fi
  fi
  [[ $LASTWIDGET == *history* || $LASTWIDGET == *line-or-beginning-search* ]] && flags+=h
  [[ $flags == *h* ]] && _ziro_clear
  (( _ziro_explicit )) && flags+=x
  (( _ziro_inflight < 32 )) || return
  (( _ziro_gen++ ))
  if _ziro_send CTX $_ziro_gen $CURSOR $COLUMNS $LINES "${KEYMAP:-main}" "$PWD" "$flags" "$BUFFER"; then
    _ziro_ui_send CTX $_ziro_gen $CURSOR $COLUMNS $LINES "${KEYMAP:-main}" "$PWD" "$flags" "$BUFFER"
    (( _ziro_inflight++ ))
    _ziro_previous_buffer=$BUFFER _ziro_previous_cursor=$CURSOR _ziro_previous_cols=$COLUMNS _ziro_navigated=0
  fi
  [[ ${ZIRO_TRACE:-0} == 1 ]] && _ziro_send TRACE 1 $(( int((EPOCHREALTIME-begin)*1000000) ))
  return 0
}
_ziro_finish() { _ziro_ui_send UI_STATE 0; _ziro_clear; _ziro_hidden=''; }
_ziro_preexec() {
  _ziro_ui_send UI_STATE 0
  _ziro_worker_stop
  local cmd=$1
  [[ -o histignorespace && $cmd == ' '* ]] && return
  [[ -n ${HISTORY_IGNORE:-} && $cmd == ${~HISTORY_IGNORE} ]] && return
  _ziro_hist_id="$_ziro_session-$EPOCHREALTIME" _ziro_hist_start=$EPOCHREALTIME
  _ziro_send HIST_START "$_ziro_hist_id" "$EPOCHREALTIME" "$PWD" "$cmd"
}
_ziro_exit() { _ziro_worker_stop; _ziro_send BYE; _ziro_disconnect; }
