#!/usr/bin/env zsh
# State regression: preserve the visual frame but reject stale insertion.
emulate -R zsh
setopt extendedglob
autoload -Uz is-at-least
local root=${0:A:h:h}
source "$root/plugin/lib/overlay.zsh"
typeset -gi _ziro_overlay_mode=0
source "$root/plugin/lib/render.zsh"
source "$root/plugin/lib/keys.zsh"
typeset -ga region_highlight=() _ziro_buffers=() _ziro_cursors=() _ziro_row_starts=() _ziro_row_ends=() _ziro_highlights=() _ziro_base_highlights=()
typeset -gi _ziro_visible=0 _ziro_stale=0 _ziro_selected=1 _ziro_ghost_len=0 _ziro_navigated=0
BUFFER='cd '
_ziro_frame FRAME 1 3 0 1 $'xyz\nMENU' 1 '3 8 fg=4' 4 8 'cd wrong/' 9
[[ $_ziro_visible == 1 && $_ziro_ghost == xyz ]] || exit 1
BUFFER='cd A'
_ziro_invalidate
[[ $POSTDISPLAY == $'\nMENU' && $_ziro_visible == 1 && $_ziro_stale == 1 ]] || exit 2
[[ $_ziro_row_starts[1] == 1 && $_ziro_row_ends[1] == 5 && $_ziro_ghost_len == 0 ]] || exit 3
_ziro_accept && exit 4
[[ $BUFFER == 'cd A' ]] || exit 5
_ziro_clear
[[ -z $POSTDISPLAY && $_ziro_visible == 0 && $_ziro_stale == 0 ]] || exit 6
print 'PASS: pending edit retains menu, removes stale ghost, shifts spans, rejects stale insertion'
