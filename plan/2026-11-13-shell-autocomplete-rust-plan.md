# План реализации MVP ziro

**Цель:** меню подсказок в стиле Kiro прямо в zsh: кандидаты из живого compsys и истории, ghost-text из истории, Rust-демон ранжирует и раскладывает меню.

**ТЗ:** `../2026-11-13-shell-autocomplete-rust-design.md` (ссылки §N ведут туда).

**Корень репозитория:** `~/Desktop/balovstvo/ziro`. В систему ничего не ставим, `~/.zshrc` не трогаем.

**Стек:** Rust stable, tokio (current-thread), rusqlite (bundled), nucleo-matcher, unicode-width, serde + toml, clap, tracing. zsh ≥ 5.8 с `zsh/zpty`, `zsh/net/socket`, `zsh/system`, `zsh/datetime`.

## Правила работы

- Тесты пишем там, где логика хитрая и чистая: кодек протокола, импорт истории, ранжирование, раскладка, вставка с экранированием. Без test-first, без тестов ради покрытия.
- zsh-часть проверяем руками во временном окружении:
  ```sh
  ZDOTDIR=$(mktemp -d)  # внутри .zshrc: source ~/.oh-my-zsh/oh-my-zsh.sh с теми же plugins=()
                        # и последней строкой source <repo>/plugin/ziro.plugin.zsh
  ```
- После каждой задачи: `cargo clippy --all-targets -- -D warnings` и `cargo test` зелёные, коммит.
- Ограничения §2 действуют всегда, особенно: ничего блокирующего на нажатие клавиши (никаких `$(…)`, внешних команд, блокирующих `read`).

## Структура

```
Cargo.toml                       workspace
crates/ziro-proto/               сообщения v1, encode/decode, разбор потока по \0
crates/ziro-core/src/
  config.rs                      Config, значения по умолчанию из §9
  candidate.rs                   Candidate, Kind, разбор CANDS и описаний
  history.rs                     SQLite, импорт HISTFILE, frecency
  rank.rs                        rank(), ghost(), should_show()
  layout.rs                      раскладка меню → кадр
  apply.rs                       итоговый BUFFER/CURSOR при вставке
crates/ziro/src/
  main.rs                        clap: daemon | init zsh | doctor | stats
  daemon/                        сервер, сессии, связь с воркером, lock
plugin/
  ziro.plugin.zsh                вход, проверки, загрузка lib/*
  lib/ipc.zsh                    подключение, отправка, приём кадров
  lib/render.zsh                 POSTDISPLAY, region_highlight
  lib/hooks.zsh                  precmd/preexec/line-*
  lib/worker.zsh                 zpty-воркер, перехват compadd
  lib/keys.zsh                   диспетчер клавиш
  lib/compat.zsh                 zsh-autosuggestions, vi-mode
spikes/                          Task 0
```

---

## Task 0. Spike: постоянный zpty-воркер

Самое рискованное место (§5.4, §12), проверяем первым.

- `spikes/worker-spike.zsh`: compinit → `zpty -b` форкает воркер → внутри `vared` с виджетом-циклом, который читает `<cursor>\x1f<buffer>`, ставит `BUFFER/CURSOR`, вызывает completion-виджет с перехватом `compadd` (§5.3) и печатает записи `C␟…` + `END`.
- Прогнать `'git ch' 'git checkout ' 'tar -' 'ls ~/De'` в **одном** воркере.
- Замерить форк и время ответа (`EPOCHREALTIME`), проверить, что функции и `compdef` из родителя видны, и что при 2000 запросах вывод pty дренируется без блокировки.
- Числа и выводы — в `spikes/RESULTS.md`. Сохранить реальные строки `-d` от git/tar/ssh как фикстуры для разбора описаний.

**Готово, когда:** все 4 запроса дают кандидатов с описаниями в одном воркере. Если цикл не работает — фиксируем fallback (новый zpty на запрос) и правим Task 7.

## Task 1. Workspace и протокол

- `git init`, workspace, `rust-toolchain.toml`, `.gitignore`.
- `ziro-proto`: enum'ы `ShellMsg`, `WorkerMsg`, `DaemonMsg` строго по §5.7; `encode`/`decode`; `Framer` режет поток по `\0`, держит хвост, лимит сообщения 1 МБ.
- Тесты: roundtrip всех вариантов, точная байтовая раскладка `FRAME`, сообщение кусками по байту, буфер с кириллицей и `\n`.

## Task 2. История

- `HistoryStore` на rusqlite (WAL): `record_start/record_end`, `import_histfile` (extended и простой формат, `\`-продолжения, метафицированные байты zsh, повторный импорт не дублирует).
- `prefix_matches(prefix, cwd, now, limit)` и `arg_usage(cmd_words, cwd, now)` с frecency из §5.8.
- Тесты: оба формата, многострочные, кириллица, идемпотентность, веса cwd/exit, `arg_usage` по позиции.

## Task 3. Конфиг и кандидаты

- `Config` с дефолтами §9, отсутствующий файл → дефолты, неизвестный ключ → предупреждение.
- `Candidate`, `Kind`, `from_batch`, `split_description` (по первому `--`), `kind_of(tag, flags, word)`.
- Тесты: `split_description` на фикстурах из Task 0, таблица `kind_of`.

## Task 4. Ранжирование и ghost

- `Query::parse` (слова с учётом кавычек и `\` — для ключа кэша и буста), `rank`, `ghost`, `should_show` по §4.1 и §5.8.
- Тесты: префикс выше подпоследовательности, стабильность при равенстве, буст веток из истории, не больше 2 строк истории, ghost только с курсором в конце, правила `should_show`. Проверить, что 1000 кандидатов ранжируются < 1 мс в release.

## Task 5. Раскладка и вставка

- `layout` → `postdisplay`, `ghost_len`, спаны подсветки, диапазоны строк, счётчик. Смещения в символах, ширина через `unicode-width`. Рамка, иконки, окно из `max_items` вокруг выбранного.
- `apply_candidate` по §5.9.
- Тесты: ширина ≤ `cols-1` при cols 20/40/200 и длинных описаниях; сдвиг влево у правого края; кириллица/CJK/эмодзи; `My Docs/`, `'My`, `файл'1.txt`, `--color=auto`, курсор в середине слова.

## Task 6. Демон

- `ziro daemon [--detach] [--socket PATH]`: путь сокета и права по §5.6, `SO_PEERCRED`, `flock`, удаление мёртвого сокета, лог.
- Сессии: на `HELLO` → `CFG` (или `ERR proto`); на `CTX` → кадр из кэша/истории сразу, `REQ` воркеру, если контекст сменился (занят — пометить pending); на `CANDS` → отбросить устаревший `gen`, обновить кэш, кадр шеллу; таймер простоя и таймаут воркера (`ERR worker_timeout`); `KEY` за пределами окна → новый кадр.
- Несколько тестов на `UnixStream` с фейковыми шеллом и воркером: путь `CTX → REQ → CANDS → FRAME`, перефильтрация без воркера, отброс старого `gen`, схлопывание запросов при занятом воркере.

## Task 7. Плагин: IPC, рендер, хуки, воркер

- `ipc.zsh`: `zsocket`, отправка через `print -rn -u`, приём через `zle -F` + `sysread`, предохранитель inflight, EOF → выключено до `precmd`, автозапуск демона только из `precmd`.
- `render.zsh`: применить/стереть кадр, свои записи `region_highlight` (`memo=ziro`, на 5.8 — по сохранённому списку), `zle -R`.
- `hooks.zsh`: `line-pre-redraw` → `CTX` (флаг `h` по `$LASTWIDGET`), `line-finish` → стереть, `preexec`/`precmd` → история (с учётом `HIST_IGNORE_SPACE`).
- `worker.zsh` по итогам Task 0: старт в `precmd`, стоп в `preexec`/`zshexit`, пересоздание при смене `$PWD`, `SIGINT`/пересоздание по `worker_timeout`.
- Отказ от активации: `ZIRO_DISABLE=1`, `ISTERM`, zsh < 5.8.

**Готово, когда** (руками): `git ch` показывает меню с описаниями; своя функция с `compdef` из текущей сессии даёт свои аргументы; `slowcmd` с `sleep 2` не тормозит набор; после Enter меню не остаётся; `kill -9` демона — ни мусора, ни лагов, на следующем промпте всё снова работает; после 20 промптов висит не больше одного воркера.

## Task 8. Клавиши

- `_ziro_bind_all` в первом `precmd`: для клавиш из конфига в `emacs`/`viins`/`main` запомнить прежний виджет и забиндить диспетчер (§5.5).
- Локальные next/prev/accept по данным кадра, `KEY` демону асинхронно. Enter по `enter_accepts`. Esc со скрытием до правки и vi-mode. Режим `explicit` с Ctrl+Space.

**Готово, когда** (руками, с oh-my-zsh): ↓+Tab вставляет пункт и показывает следующее меню; Enter без навигации выполняет команду; ↑ на пустой строке и повторные ↑ листают историю; `git `, Esc, ↑ — prefix-поиск; `echo *.md`+Tab раскрывает глоб; в `explicit` всё как без ziro до Ctrl+Space; vi-mode: Esc скрывает меню и переходит в `vicmd`.

## Task 9. Ghost-text, совместимость, CLI

- Принятие ghost: →/End целиком, Ctrl+→ по слову — только диапазон ghost.
- `compat.zsh`: отключение zsh-autosuggestions и перехват его accept-виджетов (§5.2).
- `ziro init zsh` (печатает строку `source`), `ziro doctor` (версия zsh, модули, демон, конфликтующие плагины, порядок загрузки — `.zshrc` только читает), `ziro stats [--json]` (гистограммы §7, размер истории). `TRACE` из плагина при `ZIRO_TRACE=1`.

**Готово, когда:** с zsh-autosuggestions в `plugins=()` → принимает ghost, а выполненная команда никогда не содержит текста меню; подсветка синтаксиса не пропадает при видимом меню.

## Task 10. Приёмка

- Пройти критерии §11 руками в kitty и в tmux внутри kitty, включая файлы с пробелами/кавычками/кириллицей и курсор в середине слова.
- Поработать ~10 минут с `ZIRO_TRACE=1`, сверить `ziro stats` с бюджетами §7.
- README: установка через `ziro init zsh`, подключение после oh-my-zsh.
- Итоги и замеры дописать в `spikes/RESULTS.md`.
