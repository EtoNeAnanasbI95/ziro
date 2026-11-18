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
  if _ziro_send CTX $_ziro_gen $CURSOR $COLUMNS $LINES "${KEYMAP:-main}" "$PWD" "$flags" "$BUFFER" $_ziro_origin_col; then
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

# Where the edit buffer starts on screen (= prompt width), so the menu can sit
# under the cursor. Asked from the terminal (DSR) once per prompt and again when
# an async prompt repaints itself; keys typed ahead are pushed back into ZLE.
# A terminal that doesn't answer is not asked again.
_ziro_measure_origin() {
  emulate -L zsh; setopt extendedglob
  (( _ziro_dsr && !_ziro_overlay_mode )) && [[ -n $_ziro_fd && $LBUFFER != *$'\n'* ]] || return 0
  local buf='' c col
  # Draw first, otherwise the cursor isn't where ZLE is going to put it yet.
  zle -R
  # `read -t` inside ZLE dispatches `zle -F` handlers; detach ours meanwhile.
  zle -F $_ziro_fd 2>/dev/null
  [[ -n $_ziro_pty_fd ]] && zle -F $_ziro_pty_fd 2>/dev/null
  if print -rn -- $'\e[6n' >/dev/tty 2>/dev/null; then
    while read -rs -k 1 -t 0.15 c </dev/tty 2>/dev/null; do
      buf+=$c
      [[ $buf == *$'\e['[0-9]##\;[0-9]##R ]] && break
    done
    if [[ $buf == (#b)(*)$'\e['([0-9]##)\;([0-9]##)R ]]; then
      [[ -n $match[1] ]] && zle -U -- "$match[1]"
      col=$(( match[3] - 1 - ${(m)#LBUFFER} ))
      if (( col >= 0 && col != _ziro_origin_col )); then
        _ziro_origin_col=$col
        _ziro_previous_cursor=-1   # re-request the frame at the new position
      fi
    else
      _ziro_dsr=0
      [[ -n $buf ]] && zle -U -- "$buf"
    fi
  fi
  [[ -n $_ziro_fd ]] && zle -F $_ziro_fd _ziro_read_event
  [[ -n $_ziro_pty_fd ]] && zle -F $_ziro_pty_fd _ziro_worker_drain
  return 0
}
_ziro_line_init() {
  _ziro_origin_col=-1
  _ziro_measure_origin
}
# oh-my-zsh draws git info asynchronously via `zle .reset-prompt` after the
# prompt is shown, which changes where the buffer starts.
_ziro_wrap_async_prompt() {
  (( $+functions[_omz_async_callback] && !$+functions[_ziro_omz_async_callback] )) || return 0
  functions[_ziro_omz_async_callback]=$functions[_omz_async_callback]
  _omz_async_callback() {
    _ziro_omz_async_callback "$@"
    local ret=$?
    zle && _ziro_measure_origin && (( _ziro_previous_cursor < 0 )) && [[ -n $BUFFER ]] && _ziro_redraw
    return ret
  }
}
