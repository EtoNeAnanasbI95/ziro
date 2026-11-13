_ziro_fallback() {
  local prior=${_ziro_original[${KEYMAP}:${KEYS}]:-${_ziro_original[main:${KEYS}]:-${_ziro_original[emacs:${KEYS}]}}}
  [[ -n $prior && $prior != _ziro_* ]] && zle "$prior"
}
_ziro_accept() {
  (( _ziro_visible )) || return 1
  BUFFER=$_ziro_buffers[_ziro_selected] CURSOR=$_ziro_cursors[_ziro_selected]
  _ziro_clear
  _ziro_hidden='' _ziro_previous_cursor=-1 _ziro_navigated=0
}
_ziro_dispatch() {
  local action=$1
  if [[ $action == ghost || $action == ghost_word ]]; then
    if [[ -n $_ziro_ghost && $CURSOR == ${#BUFFER} ]]; then
      local text=$_ziro_ghost
      if [[ $action == ghost_word ]]; then
        text=''; local c started=0
        for c in ${(s::)_ziro_ghost}; do
          if [[ $c == [[:space:]] ]]; then (( started )) && break; else started=1; fi
          text+=$c
        done
      fi
      BUFFER+=$text CURSOR=${#BUFFER}; _ziro_clear; return
    fi
    _ziro_fallback; return
  fi
  if [[ $action == enter_menu ]]; then _ziro_explicit=1; _ziro_hidden=''; _ziro_previous_cursor=-1; _ziro_redraw; return; fi
  if [[ $action == dismiss && $_ziro_visible == 1 ]]; then _ziro_hidden=$BUFFER; _ziro_send KEY $_ziro_gen dismiss; _ziro_clear; [[ $KEYMAP == viins ]] && _ziro_fallback; return; fi
  if (( !_ziro_visible )) || [[ $_ziro_arrows == explicit && $_ziro_explicit == 0 ]]; then _ziro_fallback; return; fi
  case $action in
    accept) _ziro_accept ;;
    enter) if [[ $_ziro_enter_accepts == always || ( $_ziro_enter_accepts == explicit && $_ziro_navigated == 1 ) ]]; then _ziro_accept; else _ziro_clear; _ziro_fallback; fi ;;
    next|prev|page_up|page_down)
      _ziro_navigated=1
      case $action in
        next) (( _ziro_selected < $#_ziro_buffers )) && (( _ziro_selected++ )) ;;
        prev) (( _ziro_selected > 1 )) && (( _ziro_selected-- )) ;;
      esac
      _ziro_send KEY $_ziro_gen "$action"; _ziro_paint; zle -R ;;
    *) _ziro_fallback ;;
  esac
  return 0
}
_ziro_bind() {
  local key=$1 action=$2 map prior widget="_ziro_key_$2"
  for map in emacs viins main; do
    prior=${${(z)"$(bindkey -M $map "$key" 2>/dev/null)"}[-1]}
    if [[ $prior == _ziro_* ]]; then
      _ziro_original[$map:${(g:ce:)key}]=${_ziro_original[emacs:${(g:ce:)key}]:-${_ziro_original[viins:${(g:ce:)key}]}}
      continue
    fi
    _ziro_original[$map:${(g:ce:)key}]=$prior
    bindkey -M $map "$key" "$widget" 2>/dev/null
  done
}
_ziro_bind_all() {
  (( _ziro_bound )) && return
  _ziro_bound=1
  local action key
  for action in next prev accept enter dismiss enter_menu ghost ghost_word page_up page_down; do
    functions[_ziro_key_$action]="_ziro_dispatch $action"
    zle -N _ziro_key_$action
  done
  local accept=${1:-'^I'} enter=${2:-'^@'} dismiss=${3:-'^['}
  shift $(( $# >= 3 ? 3 : $# ))
  _ziro_bind "$accept" accept; _ziro_bind "$enter" enter_menu; _ziro_bind "$dismiss" dismiss
  _ziro_bind '^M' enter; _ziro_bind '^[[C' ghost; _ziro_bind '^[OC' ghost; _ziro_bind '^[[F' ghost; _ziro_bind '^[[1;5C' ghost_word
  _ziro_bind '^[[5~' page_up; _ziro_bind '^[[6~' page_down
  action=next
  for key in "$@"; do [[ $key == '|' ]] && { action=prev; continue; }; _ziro_bind "$key" "$action"; done
}
