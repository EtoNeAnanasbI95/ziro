_ziro_remove_highlights() {
  if is-at-least 5.9; then
    region_highlight=("${(@)region_highlight:#*memo=ziro*}")
  else
    local -a retained=(); local item
    for item in "${region_highlight[@]}"; do (( ${_ziro_highlights[(Ie)$item]} )) || retained+=("$item"); done
    region_highlight=("${retained[@]}")
  fi
  _ziro_highlights=()
}
_ziro_clear() {
  POSTDISPLAY=''
  _ziro_remove_highlights
  _ziro_visible=0 _ziro_ghost='' _ziro_ghost_len=0
  _ziro_buffers=() _ziro_cursors=() _ziro_base_highlights=()
}
_ziro_frame() {
  local -a f=("$@")
  _ziro_remove_highlights
  local prior_selected=$_ziro_selected prior_buffers="${(pj:\x1f:)_ziro_buffers}"
  _ziro_ghost_len=$f[3] _ziro_selected=$(( f[4]+1 ))
  local rows=$f[5] nhl=$f[7] i offset=8
  POSTDISPLAY=$f[6]
  _ziro_ghost=${POSTDISPLAY[1,$_ziro_ghost_len]}
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
  if (( _ziro_navigated )) && [[ $prior_buffers == "${(pj:\x1f:)_ziro_buffers}" ]]; then _ziro_selected=$prior_selected; fi
  _ziro_paint
}
_ziro_paint() {
  _ziro_remove_highlights
  local spec start end style memo=''
  is-at-least 5.9 && memo=' memo=ziro'
  for spec in "${_ziro_base_highlights[@]}"; do
    start=${spec%% *}; spec=${spec#* }; end=${spec%% *}; style=${spec#* }
    [[ $style == *bg=* ]] && continue
    _ziro_highlights+=("$(( ${#BUFFER}+start )) $(( ${#BUFFER}+end )) $style$memo")
  done
  if (( _ziro_visible )); then
    _ziro_highlights+=("$(( ${#BUFFER}+_ziro_row_starts[_ziro_selected] )) $(( ${#BUFFER}+_ziro_row_ends[_ziro_selected] )) bg=4,fg=15$memo")
  fi
  region_highlight+=("${_ziro_highlights[@]}")
}
