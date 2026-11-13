#!/usr/bin/env zsh
# The probe uses the production worker and a fake daemon over a Unix socket.
# No startup files are sourced or modified.
exec python3 "${0:A:h:h}/tests/worker_probe.py" "$@"
