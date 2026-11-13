# ziro

Меню живых подсказок zsh и ghost-text из истории. Кандидаты собирает compsys
в отдельном zpty-процессе, Rust-демон хранит историю, ранжирует подсказки и
готовит кадры для ZLE. Текст меню находится в `POSTDISPLAY`, отдельно от команды.

Текущий статус: рабочий экспериментальный MVP; полная приёмка и все бюджеты
задержек ещё не подтверждены. Подробности: [результаты](spikes/RESULTS.md).

## Запуск из репозитория

Нужны Linux, Rust stable и zsh ≥ 5.8. Рекомендован zsh 5.9.

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"
ziro init zsh
```

Последняя команда **печатает** строку `source`. Добавьте её в конец `.zshrc`
после oh-my-zsh и подсветчиков или выполните в текущей оболочке:

```zsh
eval "$(ziro init zsh)"
```

Автоматически `.zshrc` не меняется. Для пробного запуска можно использовать
временный `ZDOTDIR`. Внутри inshellisense (`ISTERM` задан) плагин отключается;
для отдельной тестовой оболочки запустите `env -u ISTERM zsh`.

Демон автоматически запускается на новом промпте. Ручной запуск:

```sh
ziro daemon --detach
ziro doctor
ziro stats --json
```

Сокет: `$XDG_RUNTIME_DIR/ziro/daemon.sock`, иначе `/tmp/ziro-$UID/daemon.sock`.
История и лог: `${XDG_STATE_HOME:-$HOME/.local/state}/ziro/`.
`HISTFILE` передаётся при автоматическом старте; импорт не дублирует записи.

## Управление

- ↑/↓, Shift+Tab: выбор строки меню; Tab: вставка.
- Enter: вставка после явной навигации, иначе обычное выполнение.
- Esc: скрытие меню до изменения буфера.
- →, End: принятие ghost-text; Ctrl+→: принятие следующего слова.
- PgUp/PgDn: прокрутка. Ctrl+Space: явный вход в меню.
- Если меню скрыто, вызываются прежние виджеты клавиш.

`ZIRO_DISABLE=1` отключает подключение. `ZIRO_BIN` и `ZIRO_SOCKET` задают
бинарь и сокет для изолированного запуска. `ZIRO_TRACE=1` включает замеры
обработки изменений строки; `ZIRO_LOG=debug` включает подробный лог демона.

ziro отключает zsh-autosuggestions, поскольку оба плагина используют
`POSTDISPLAY`. С zsh-autocomplete одновременно подключать его не следует.

## Конфиг

`~/.config/ziro/config.toml` (или `$XDG_CONFIG_HOME/ziro/config.toml`):

```toml
[menu]
max_items = 8
enter_accepts = "explicit" # explicit | always | never

[keys]
arrows = "menu"           # menu | explicit

[worker]
timeout_ms = 500
refresh_idle_ms = 80

[ghost]
enable = true
```

Полные значения по умолчанию перечислены в [ТЗ, §9](2026-11-13-shell-autocomplete-rust-design.md).
`ziro reload` / SIGHUP перечитывает конфиг демона. После изменения биндингов
откройте новую оболочку. Неизвестные ключи попадают в лог как предупреждения.

## Проверка

Все интеграционные проверки создают временные rc-файлы, сокеты и базы;
пользовательский `.zshrc` не изменяют.

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
python3 tests/daemon_smoke.py
python3 tests/shell_smoke.py
ZIRO_TEST_OMZ=1 python3 tests/shell_smoke.py
ZIRO_TEST_TMUX=1 python3 tests/shell_smoke.py
ZIRO_STRESS=1 python3 tests/worker_probe.py
cargo run --release -p ziro-core --example rank_bench
```

Проверка oh-my-zsh предполагает установленный `~/.oh-my-zsh` с плагинами
`git`, `zsh-syntax-highlighting`, `fast-syntax-highlighting`, `zsh-autosuggestions`.
