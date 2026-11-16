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
and `env/`. Set `WHURL_WORKSPACE` to use one workspace from anywhere. The env defaults to `$HURL_ENV`, then `local`.

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

`enter` run · `/` filter · `e`/`E` env · `t` asserts · `tab`/`1-5` Body, Headers, Request,
Timing, Source · `[ ]` entries in a chained file · `space`, `ctrl-d/u`, `J/K`, `h/l` scroll ·
`x`/`X` export Markdown/JSON to `.whurl/exports/` · `o` open in `$EDITOR` · `R` reload · `?` help.

The summary shows the latency waterfall (dns, connect, tls, send, wait, download) and a sparkline
of the last 30 runs of the request in the current env; the Timing tab adds per-phase bars and
min/p50/p90/max.

## Secrets

Hurl redacts secrets in its terminal output but not in its JSON report, which whurl parses. whurl
masks every value from the env's secrets files as `***` before printing, storing, or exporting.
Values a response returns (a freshly minted token, say) aren't known secrets and are stored as-is
in `.whurl/runs/`.
