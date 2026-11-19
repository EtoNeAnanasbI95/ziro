# Runs in a fork of the user's shell, never in the foreground editor.
_ziro_worker_send() {
  local _ziro_wire="${(pj:\x1f:)@}"
  [[ $_ziro_wire != *$'\x1e'* ]] || return 1
  print -rn -u $_ziro_worker_fd -- "${_ziro_wire}"$'\0'
}
_ziro_capture() {
  unset 'compstate[vared]'
  typeset -ga _ziro_records=()
  typeset -g _ziro_prefix=$PREFIX _ziro_suffix=$SUFFIX _ziro_iprefix=$IPREFIX _ziro_isuffix=$ISUFFIX _ziro_quote=${compstate[quote]}
  compadd() {
    local -a _ziro_matches _ziro_display _ziro_args
    local _ziro_group='' _ziro_tag=${curtag:-} _ziro_dname='' _ziro_p='' _ziro_s='' _ziro_flags='' _ziro_opt _ziro_i
    # Parse options only: stop at `-`/`--` or the first word. Callers like
    # _yay pass ~150k package names here, so never walk the whole argv.
    for (( _ziro_i=1; _ziro_i <= $#; _ziro_i++ )); do
      _ziro_opt=${argv[_ziro_i]}
      [[ $_ziro_opt == -(|-) || $_ziro_opt != -* ]] && break
      [[ $_ziro_opt == *f* ]] && _ziro_flags+=f
      [[ $_ziro_opt == *q* ]] && _ziro_flags+=q
      case $_ziro_opt in
        # Output-array probes: completion helpers use them internally.
        -[ADO]|-?*[ADO]) builtin compadd "$@"; return ;;
        -d|-?*d) (( _ziro_i++ )); _ziro_dname=${argv[_ziro_i]} ;;
        -[JV]|-?*[JV]) (( _ziro_i++ )); _ziro_group=${argv[_ziro_i]} ;;
        -[Pp]|-?*[Pp]) (( _ziro_i++ )); _ziro_p+=${argv[_ziro_i]} ;;
        -[Ss]|-?*[Ss]) (( _ziro_i++ )); _ziro_s+=${argv[_ziro_i]} ;;
        # -o takes an optional order list (nosort, match, numeric, reverse).
        -o|-?*o) [[ ${argv[_ziro_i+1]} == (match|nosort|numeric|reverse)* ]] && (( _ziro_i++ )) ;;
        # Remaining options that take an argument: skip it.
        -[iIWXxrRFME]|-?*[iIWXxrRFME]) (( _ziro_i++ )) ;;
      esac
    done
    _ziro_args=("$@")
    [[ -n $_ziro_dname ]] && _ziro_display=("${(@P)_ziro_dname}")
    builtin compadd -A _ziro_matches "${_ziro_args[@]}"
    local _ziro_ret=$?
    _ziro_prefix=$PREFIX _ziro_suffix=$SUFFIX _ziro_iprefix=$IPREFIX _ziro_isuffix=$ISUFFIX _ziro_quote=${compstate[quote]}
    # Index descriptions once: a linear search per match is quadratic and
    # dominated command-name completion (hundreds of matches).
    local -A _ziro_dexact _ziro_dfirst
    local _ziro_word _ziro_desc _ziro_key _ziro_word_tag _ziro_path
    local -a _ziro_hits
    for _ziro_desc in "${_ziro_display[@]}"; do
      [[ -n $_ziro_desc ]] || continue
      (( ${+_ziro_dexact[$_ziro_desc]} )) || _ziro_dexact[$_ziro_desc]=$_ziro_desc
      _ziro_key=${_ziro_desc%%[[:space:]]*}
      [[ -n $_ziro_key ]] && (( ! ${+_ziro_dfirst[$_ziro_key]} )) && _ziro_dfirst[$_ziro_key]=$_ziro_desc
    done
    local -i _ziro_same=$(( $#_ziro_display == $#_ziro_matches )) _ziro_first=$(( CURRENT == 1 ))
    for (( _ziro_i=1; _ziro_i <= $#_ziro_matches; _ziro_i++ )); do
      _ziro_word=${(Q)_ziro_matches[_ziro_i]} _ziro_desc=''
      if (( $#_ziro_display )) && [[ -n $_ziro_word ]]; then
        _ziro_desc=${_ziro_dexact[$_ziro_word]:-${_ziro_dfirst[$_ziro_word]:-}}
        if [[ -z $_ziro_desc && $_ziro_word == *[[:space:]]* ]]; then
          # Words with blanks: "word<space>description" cannot be keyed.
          _ziro_hits=(${(M)_ziro_display:#${_ziro_word}[[:space:]]*})
          _ziro_desc=${_ziro_hits[1]:-}
        fi
        (( _ziro_same )) && [[ -z $_ziro_desc ]] && _ziro_desc=${_ziro_display[_ziro_i]}
      fi
      if [[ $_ziro_flags == *f* ]]; then
        _ziro_path="${_ziro_p}${_ziro_word}"
        [[ $_ziro_path == '~/'* ]] && _ziro_path="$HOME/${_ziro_path#'~/'}"
        if [[ $_ziro_word != */ && -d $_ziro_path ]]; then _ziro_word+=/; fi
      fi
      [[ $_ziro_word == *[$'\0\x1f\x1e']* || $_ziro_desc == *[$'\0\x1f\x1e']* ]] && continue
      _ziro_word_tag=$_ziro_tag
      if (( _ziro_first )); then
        if (( $+aliases[$_ziro_word] )); then _ziro_word_tag=aliases
        elif (( $+functions[$_ziro_word] )); then _ziro_word_tag=functions
        elif (( $+builtins[$_ziro_word] )); then _ziro_word_tag=builtins
        elif (( $+reswords[$_ziro_word] )); then _ziro_word_tag=reserved-words
        elif (( $+commands[$_ziro_word] )); then _ziro_word_tag=commands
        fi
      elif [[ $_ziro_word_tag == *commands* ]]; then
        _ziro_word_tag=subcommands
      fi
      _ziro_records+=("C${_ziro_us}${_ziro_group}${_ziro_us}${_ziro_word_tag}${_ziro_us}${_ziro_word}${_ziro_us}${_ziro_desc}${_ziro_us}${_ziro_us}${_ziro_flags}${_ziro_us}${_ziro_p}${_ziro_us}${_ziro_s}")
    done
    # Let compsys observe matches, while suppressing insertion/listing below.
    builtin compadd "${_ziro_args[@]}"
    return $_ziro_ret
  }
  { _main_complete } always { unfunction compadd }
  compstate[list]=''
  compstate[insert]=''
  _ziro_worker_send CANDS $_ziro_worker_gen "$_ziro_prefix" "$_ziro_suffix" "$_ziro_iprefix" "$_ziro_isuffix" "$_ziro_quote" $#_ziro_records
  local _ziro_record
  for _ziro_record in "${_ziro_records[@]}"; do print -rn -u $_ziro_worker_fd -- "$_ziro_record"$'\0'; done
  _ziro_worker_send END $_ziro_worker_gen
}
_ziro_worker_read() {
  local _ziro_chunk _ziro_record
  sysread -i $_ziro_worker_fd -s 65536 -t 0 _ziro_chunk || { [[ $? == 5 ]] && { zle -F $_ziro_worker_fd; zle .accept-line; }; return; }
  _ziro_worker_process_chunk "$_ziro_chunk"
}
_ziro_worker_process_chunk() {
  local _ziro_record
  _ziro_worker_tail+=$1
  # One split instead of ${tail%%$'\0'*}: that pattern is quadratic in zsh.
  local -a _ziro_records=("${(@ps:\0:)_ziro_worker_tail}")
  _ziro_worker_tail=$_ziro_records[-1]; _ziro_records[-1]=()
  for _ziro_record in "${_ziro_records[@]}"; do
    local -a _ziro_fields=("${(@ps:\x1f:)_ziro_record}")
    [[ $_ziro_fields[1] == REQ ]] || continue
    _ziro_worker_gen=$_ziro_fields[2]
    BUFFER=$_ziro_fields[4] CURSOR=$_ziro_fields[3]
    zle _ziro_complete
    BUFFER=''
  done
}
_ziro_worker_trigger() { zle _ziro_complete; BUFFER=''; }
_ziro_worker_init() {
  zsocket "$_ziro_socket" || return
  local _ziro_low_fd=$REPLY
  exec {_ziro_worker_fd}>&$_ziro_low_fd
  exec {_ziro_low_fd}>&-
  _ziro_worker_send WORKER_HELLO "$_ziro_session" "$PWD"
  zle -N _ziro_worker_read
  zle -F -w $_ziro_worker_fd _ziro_worker_read
}
_ziro_worker_main() {
  setopt extendedglob
  _ziro_zwc_fpath
  (( COLUMNS > 0 )) || COLUMNS=80
  (( LINES > 0 )) || LINES=24
  typeset -ga precmd_functions=() preexec_functions=() zshexit_functions=()
  unfunction zshexit 2>/dev/null
  if [[ -n ${_ziro_ui_fd:-} ]]; then exec {_ziro_ui_fd}>&-; _ziro_ui_fd=''; fi
  # The fork inherits every fd handler of the editor (oh-my-zsh async git
  # prompt, other plugins). Left installed, the worker would read the same
  # pipes and steal bytes from the parent's handlers, corrupting the prompt.
  local _ziro_line _ziro_hfd
  local -a _ziro_words
  for _ziro_line in "${(@f)$(zle -F -L 2>/dev/null)}"; do
    _ziro_words=(${(z)_ziro_line})
    _ziro_hfd=${_ziro_words[(r)<->]}
    [[ -n $_ziro_hfd ]] || continue
    zle -F $_ziro_hfd 2>/dev/null
    exec {_ziro_hfd}<&- 2>/dev/null
  done
  if [[ -n ${_ziro_fd:-} ]]; then exec {_ziro_fd}>&- 2>/dev/null; _ziro_fd=''; fi
  zle -N zle-line-pre-redraw _ziro_worker_noop
  zle -N zle-line-finish _ziro_worker_noop
  zle -N zle-keymap-select _ziro_worker_noop
  local _ziro_worker_fd _ziro_worker_tail='' _ziro_worker_gen=0 _ziro_empty=''
  zle -C _ziro_complete .complete-word _ziro_capture
  zle -N _ziro_worker_trigger
  bindkey -e
  bindkey '^]' _ziro_worker_trigger
  zle -N zle-line-init _ziro_worker_init
  if zle; then
    # A restart from an fd callback inherits an active editor. Completion
    # widgets can use that editor directly; nested vared is forbidden.
    _ziro_worker_init
    local _ziro_chunk
    while sysread -i $_ziro_worker_fd -s 65536 _ziro_chunk; do
      _ziro_worker_process_chunk "$_ziro_chunk"
    done
  else
    vared _ziro_empty
  fi
  exec {_ziro_worker_fd}>&-
}
_ziro_worker_noop() { :; }
_ziro_worker_start() {
  _ziro_worker_stop
  zpty -b ziro-worker _ziro_worker_main
  _ziro_pty_fd=$REPLY _ziro_worker_cwd=$PWD
  zle -F $_ziro_pty_fd _ziro_worker_drain 2>/dev/null
}
_ziro_worker_drain() {
  local chunk
  if [[ $2 == (hup|err|nval) ]] || ! zpty -t ziro-worker; then zle -F $_ziro_pty_fd 2>/dev/null; return; fi
  if ! zpty -r ziro-worker chunk 2>/dev/null; then
    zpty -t ziro-worker || zle -F $_ziro_pty_fd 2>/dev/null
  fi
  return 0
}
_ziro_worker_stop() {
  if [[ -n $_ziro_pty_fd ]]; then zle -F $_ziro_pty_fd 2>/dev/null; fi
  zpty -d ziro-worker 2>/dev/null
  _ziro_pty_fd=''
}
