_ziro_remove_highlights() {
  if (( _ziro_memo )); then
    region_highlight=("${(@)region_highlight:#*memo=ziro*}")
  else
    local -a retained=(); local item
    for item in "${region_highlight[@]}"; do (( ${_ziro_highlights[(Ie)$item]} )) || retained+=("$item"); done
    region_highlight=("${retained[@]}")
  fi
  _ziro_highlights=()
}
_ziro_invalidate() {
  # Keep the last visual frame until its replacement is available. Old
  # prepared buffers must never remain actionable after the edit.
  _ziro_stale=1
  _ziro_remove_highlights
  if (( _ziro_ghost_len )); then
    local shift=$_ziro_ghost_len spec start end style
    local -a kept=() parts
    POSTDISPLAY=${POSTDISPLAY[$((shift+1)),-1]}
    for spec in "${_ziro_base_highlights[@]}"; do
      parts=(${=spec}); start=$parts[1] end=$parts[2] style="${parts[3,-1]}"
      (( end <= shift )) && continue
      kept+=("$(( start-shift )) $(( end-shift )) $style")
    done
    _ziro_base_highlights=("${kept[@]}")
    local i
    for (( i=1; i<=$#_ziro_row_starts; i++ )); do
      (( _ziro_row_starts[i]-=shift, _ziro_row_ends[i]-=shift ))
    done
  fi
  _ziro_ghost='' _ziro_ghost_len=0
  _ziro_paint
}
_ziro_clear() {
  _ziro_ui_send UI_CLEAR ${_ziro_gen:-0}
  POSTDISPLAY=''
  _ziro_remove_highlights
  _ziro_visible=0 _ziro_stale=0 _ziro_ghost='' _ziro_ghost_len=0
  _ziro_buffers=() _ziro_cursors=() _ziro_base_highlights=()
}
_ziro_frame() {
  local -a f=("$@")
  _ziro_stale=0
  _ziro_remove_highlights
  local prior_selected=$_ziro_selected prior_buffers="${(pj:\x1f:)_ziro_buffers}"
  _ziro_ghost_len=$f[3] _ziro_selected=$(( f[4]+1 ))
  local rows=$f[5] nhl=$f[7] i offset=8
  POSTDISPLAY=$f[6]
  _ziro_ghost=${POSTDISPLAY[1,$_ziro_ghost_len]}
  if (( _ziro_overlay_mode )); then
    _ziro_ui_send "${f[@]}"
    POSTDISPLAY=$_ziro_ghost
  fi
  _ziro_visible=$(( rows>0 ))
  _ziro_base_highlights=()
  for (( i=0; i<nhl; i++ )); do _ziro_base_highlights+=("$f[$((offset+i))]"); done
  (( offset+=nhl ))
  _ziro_buffers=() _ziro_cursors=() _ziro_row_starts=() _ziro_row_ends=()
  for (( i=0; i<rows; i++ )); do
    _ziro_row_starts+=("$f[offset]") _ziro_row_ends+=("$f[$((offset+1))]")
    _ziro_buffers+=("$f[$((offset+2))]") _ziro_cursors+=("$f[$((offset+3))]")
    (( offset+=4 ))
  done
  if (( _ziro_navigated && !_ziro_await_select )) && [[ $prior_buffers == "${(pj:\x1f:)_ziro_buffers}" ]]; then _ziro_selected=$prior_selected; fi
  _ziro_await_select=0
  _ziro_paint
}
_ziro_paint() {
  _ziro_remove_highlights
  local spec start end style memo=''
  local -a parts
  (( _ziro_memo )) && memo=' memo=ziro'
  for spec in "${_ziro_base_highlights[@]}"; do
    parts=(${=spec}); start=$parts[1] end=$parts[2] style="${parts[3,-1]}"
    (( _ziro_overlay_mode && end>_ziro_ghost_len )) && continue
    [[ $style == *bg=* ]] && continue
    _ziro_highlights+=("$(( ${#BUFFER}+start )) $(( ${#BUFFER}+end )) $style$memo")
  done
  if (( _ziro_visible && !_ziro_overlay_mode )) && _ziro_highlighted; then
    _ziro_highlights+=("$(( ${#BUFFER}+_ziro_row_starts[_ziro_selected] )) $(( ${#BUFFER}+_ziro_row_ends[_ziro_selected] )) bg=4,fg=15$memo")
  fi
  region_highlight+=("${_ziro_highlights[@]}")
  if (( _ziro_overlay_mode && !_ziro_stale )); then _ziro_ui_send UI_SELECT $_ziro_gen $(( _ziro_selected-1 )); fi
}
