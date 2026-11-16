# whurl

"With hurl": a CLI and TUI over a workspace of [Hurl](https://hurl.dev) files, built for
a `requests/` + `env/` layout. It runs hurl with the right env and secrets files, shows status,
latency breakdown, size, headers, and body, keeps a run history, and exports runs as Markdown or
JSON for agents.

```sh
cargo install --path .        # puts `whurl` in ~/.cargo/bin
```

Requires `hurl` on `PATH`.

## Workspace

```
requests/**/*.hurl          one or more chained requests per file
env/<env>.env               --variables-file
env/<env>.secrets.env       --secrets-file
env/shared.secrets.env      --secrets-file (every env)
.whurl/                     whurl state: runs/, exports/ (self-gitignored)
```

Found via `--workspace`, `$WHURL_WORKSPACE`, the nearest ancestor of the cwd with `requests/`
and `env/`, then `workspace` in the config. The env defaults to `-e`, `$HURL_ENV`, the config's
`run.default_env`, then `local`.

## Config

`$XDG_CONFIG_HOME/whurl/config.toml` (default `~/.config/whurl/config.toml`). Optional; unknown
keys are errors.

```toml
workspace = "~/path/to/workspace"   # used when the cwd isn't inside one

[ui]
wrap = true                       # `w` toggles
auto_switch_response = true

[request]
sections = ["headers", "body", "curl"]   # display order; omit one to hide it

[response]
sections = ["headers", "body"]

[run]
default_env = "staging"
asserts = false
```

## CLI

```sh
whurl                                   # TUI (in a terminal)
whurl ls [query]                        # list requests (--json)
whurl run get_segment_size -e staging   # fuzzy name or path; -i headers, -t asserts
whurl run seg size --body | jq .        # raw body only
whurl run health -e staging --json      # full run record
whurl run create_job -x /tmp/run.md     # also export
whurl run x -- --very-verbose           # extra hurl args after --
whurl show [last|last~2|<id>|<name>]    # re-render a stored run
whurl history [query] -n 50             # alias: log
whurl export [run] -f md|json -o FILE   # agent-friendly dump (stdout without -o)
whurl envs                              # env vars; secret names only
```

A name resolves to a path, an exact request name (`semantics/get_segment_size`), a unique file
stem (`get_segment_size`), or a unique fuzzy match.

Exit status: hurl's exit code when the run fails, 2 for whurl errors.

## TUI

`tab` moves focus between the sidebar and the main pane; `↑↓`/`jk`, `g`/`G` act on the focused one
(main also: `space`, `ctrl-d/u` page, `h/l` horizontal scroll with wrap off). Anywhere: `enter` run · `/` filter ·
`e`/`E` env · `t` asserts · `1-4` Request, Response, Meta, Source · `w` wrap · `[ ]` entries in
a chained file · `x`/`X` export Markdown/JSON to `.whurl/exports/` · `o` open in `$EDITOR` ·
`R` reload · `?` all keys.

The sidebar is a folder tree that starts collapsed: `h/l` or `←→` close/open a folder (or jump to its
parent), `space`/`enter` toggle it. Filtering opens every folder holding a match. Open folders and the
last selection persist in `.whurl/state.toml`. `ui.sidebar = "flat"` shows the ranked flat list instead.

The tab stays put as you move between requests; when a run you started finishes and the request
is still selected, the view switches to Response. The summary shows the latency waterfall (dns, connect,
tls, send, wait, download) and a sparkline of the last 30 runs in the current env; Meta adds
per-phase bars, min/p50/p90/max, asserts, and captures.

## Secrets

Hurl redacts secrets in its terminal output but not in its JSON report, which whurl parses. whurl
masks every value from the env's secrets files as `***` before printing, storing, or exporting.
Values a response returns (a freshly minted token, say) aren't known secrets and are stored as-is
in `.whurl/runs/`.
