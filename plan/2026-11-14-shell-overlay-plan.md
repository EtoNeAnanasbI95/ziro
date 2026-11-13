# ziro shell Implementation Plan

> Execute inline using superpowers:executing-plans; no implementer delegation.

**Goal:** проверяемый PTY-режим с настоящим floating popup и восстановлением терминала.
**Spec:** 2026-11-14-shell-overlay-design.md
**Architecture:** libc openpty + poll loop, vt100 screen tracking, inherited UI socket,
ZLE plugin as completion/key source. Existing Rust daemon remains unchanged except UI protocol.
**Tech Stack:** Rust stable, libc, vt100, unicode-width; zsh ≥5.8.

## Global Constraints
- No ~/.zshrc changes or installation; preserve user startup and umask.
- No blocking daemon/worker waits on keystrokes; UI writes nonblocking.
- Preserve terminal/child lifecycle and alternate screen; no menu in command text.
- Original project policy: focused tests, no mandatory test-first.

## Tasks
1. Screen model and popup: crates/ziro/src/shell/{screen,popup}.rs; placement, styles,
   model restore/draw, generation/selection and byte boundaries. cargo test/clippy.
2. PTY process and terminal lifecycle: shell/{mod,terminal,startup}.rs; new CLI action,
   startup bootstrap and guardian. Verify startup, exit, termios, flags, resize.
3. Plugin UI channel: plugin/lib/overlay.zsh, render/hooks/worker integration,
   UI protocol; retain native source path. Verify real popup/cd/aliases and keys.
4. Regression suite: tests/shell_overlay.py (real PTY + terminal emulator), alternate
   screen, signals, resize, existing source tests and tmux. Release build.
5. Independent review under requesting-code-review; fix important findings, document
   usage and measured limitations. Leave changes reviewable; do not alter user setup.
