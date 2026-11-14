# Compiled fpath for the completion worker.
#
# Every worker is a fresh fork, so a big completion function (_git is ~9k
# lines, ~140ms to parse) would be parsed again after each command. The
# worker instead resolves functions through zcompile digests: one digest per
# fpath directory, in the same order, so resolution is unchanged. The user's
# shell keeps its own fpath; digests are built in the background.

typeset -g _ziro_zwc_dir=${XDG_CACHE_HOME:-$HOME/.cache}/ziro/zwc

# Digest stem for a directory: "/usr/share/x" -> "$_ziro_zwc_dir/%usr%share%x".
_ziro_zwc_stem() { REPLY=$_ziro_zwc_dir/${1//\//%}; }

# Worker side: swap each fpath entry for its digest when it is current.
_ziro_zwc_fpath() {
  [[ -d $_ziro_zwc_dir ]] || return 0
  local dir
  local -a swapped=()
  for dir in "${fpath[@]}"; do
    _ziro_zwc_stem "$dir"
    # A digest older than its directory misses added/removed files.
    if [[ -r $REPLY.zwc && ! $dir -nt $REPLY.zwc ]]; then
      swapped+=("$REPLY")
    else
      swapped+=("$dir")
    fi
  done
  fpath=("${swapped[@]}")
}

# Parent side: refresh stale digests once per shell session, detached and
# niced, so neither startup nor typing waits for zcompile.
_ziro_zwc_refresh() {
  (( _ziro_zwc_started )) && return
  _ziro_zwc_started=1
  [[ ${ZIRO_ZWC:-1} == 1 ]] || return 0
  (
    builtin cd -q / 2>/dev/null
    setopt extendedglob nullglob
    zmodload zsh/system 2>/dev/null
    mkdir -p -m 700 -- "$_ziro_zwc_dir" 2>/dev/null || exit 0
    # One builder at a time across shells.
    : >>| "$_ziro_zwc_dir/.lock" 2>/dev/null
    zsystem flock -t 0 "$_ziro_zwc_dir/.lock" 2>/dev/null || exit 0
    local dir tmp
    local -a files newest
    for dir in "${fpath[@]}"; do
      [[ -d $dir && $dir == /* ]] || continue
      _ziro_zwc_stem "$dir"
      files=("$dir"/^*.zwc(N.))
      (( $#files )) || continue
      newest=("$dir"/^*.zwc(N.om[1]))
      [[ -r $REPLY.zwc && ! $dir -nt $REPLY.zwc && ! $newest[1] -nt $REPLY.zwc ]] && continue
      tmp=$REPLY.$$.tmp.zwc
      # -U like compinit's autoload -U; a file that fails to compile keeps
      # the whole directory on its plain sources.
      if nice -n 10 zsh -fc 'zcompile -U "$@"' ziro-zwc "$tmp" "${files[@]}" 2>/dev/null; then
        mv -f -- "$tmp" "$REPLY.zwc"
      else
        rm -f -- "$tmp" "$REPLY.zwc"
      fi
    done
  ) &!
}
typeset -gi _ziro_zwc_started=0
