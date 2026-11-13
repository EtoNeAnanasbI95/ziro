# Source after oh-my-zsh and completion/highlighting plugins.
[[ -o interactive && ${ZIRO_DISABLE:-0} != 1 && -z ${ISTERM:-} ]] || return 0
autoload -Uz is-at-least add-zsh-hook add-zle-hook-widget
is-at-least 5.8 || return 0
(( $+functions[_ziro_precmd] )) && return 0
zmodload zsh/mathfunc && zmodload zsh/net/socket && zmodload zsh/system && zmodload zsh/zpty && zmodload zsh/datetime && zmodload zsh/zselect || return 0
typeset -g _ziro_root=${${(%):-%N}:A:h} _ziro_us=$'\x1f'
typeset -g _ziro_binary=${ZIRO_BIN:-${commands[ziro]:-${_ziro_root:h}/target/debug/ziro}}
typeset -g _ziro_socket=${ZIRO_SOCKET:-${XDG_RUNTIME_DIR:-/tmp/ziro-$UID}${XDG_RUNTIME_DIR:+/ziro}/daemon.sock}
typeset -g _ziro_session="$$-$EPOCHREALTIME" _ziro_fd='' _ziro_tail='' _ziro_out='' _ziro_pty_fd='' _ziro_worker_cwd=''
typeset -g _ziro_previous_buffer='' _ziro_previous_cursor=-1 _ziro_previous_cols=0 _ziro_hidden='' _ziro_ghost='' _ziro_history_buffer='' _ziro_hist_id='' _ziro_hist_start=0
typeset -gi _ziro_gen=0 _ziro_inflight=0 _ziro_visible=0 _ziro_stale=0 _ziro_selected=1 _ziro_navigated=0 _ziro_explicit=0 _ziro_bound=0 _ziro_ghost_len=0 _ziro_history_nav=0 _ziro_history_search_cursor=0
typeset -g _ziro_ui_fd='' _ziro_ui_out=''
typeset -gi _ziro_overlay_mode=0
typeset -g _ziro_arrows=menu _ziro_enter_accepts=explicit
typeset -ga _ziro_buffers=() _ziro_cursors=() _ziro_row_starts=() _ziro_row_ends=() _ziro_highlights=() _ziro_base_highlights=() _ziro_cfg_keys=()
typeset -gA _ziro_original=()
for _ziro_lib in overlay ipc render worker hooks keys compat; do source "$_ziro_root/lib/$_ziro_lib.zsh"; done
unset _ziro_lib
_ziro_overlay_init
_ziro_compat
zle -N _ziro_read
add-zsh-hook precmd _ziro_precmd
add-zsh-hook preexec _ziro_preexec
add-zsh-hook zshexit _ziro_exit
_ziro_add_hook() {
  local hook="zle-$1" fn=$2
  local -a existing
  if zstyle -a "$hook" widgets existing; then
    # A highlighter may wrap an existing azhw dispatcher. Re-registering
    # that dispatcher would capture its own wrapper and create a cycle.
    zle -N "$fn"
    [[ ${(j: :)existing} == *":$fn"* ]] || zstyle "$hook" widgets "${existing[@]}" "9999:$fn"
  else
    add-zle-hook-widget "$1" "$fn"
  fi
}
_ziro_add_hook line-pre-redraw _ziro_redraw
_ziro_add_hook line-finish _ziro_finish
