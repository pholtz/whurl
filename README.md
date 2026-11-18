# whurl

An ergonomic CLI and TUI over your [Hurl](https://hurl.dev) files.

If you are willing to structure your hurl files and environment files, then `whurl` makes it easier to invoke them. Since it delegates to hurl under the hood, this is mainly for ease of use when making requests by hand. For programmatic use cases, you should probably just continue to use `hurl` directly. I originally built this as a replacement for postman, after becoming increasingly annoyed with it. Hurl files are extremely easy to get started with, and easy to use programmatically (for example, in CI) but when you want to use the same files during development, the experience becomes a bit more clumsy. Initially, this leads to small scripts or programs to make this less verbose -- `whurl` attempts to improve that bespoke experience.

Colloquially, `whurl` stands for `with hurl`, but you're welcome to make up your own definition.

Note: expects `hurl` on your system `PATH`.

```sh
# Build and install
cargo build --release
cargo install --path .

# You can tell whurl where your workspace is in several ways:
# whurl list                                <-- inside of an existing workspace
# whurl --workspace ~/my-workspace/         <-- with each request
# export WHURL_WORKSPACE='~/my-workspace/`  <-- once per session
# vim ~/.config/whurl/config.toml           <-- once per machine

# Usage
whurl list
whurl run petstore/get_all_pets --env production
whurl show
whurl history
whurl tui
```

## Shell completion

`whurl completions <shell>` prints a registration script for Bash, Zsh, Fish, PowerShell, or
Elvish. Source it from your shell startup file:

```sh
# Bash
source <(whurl completions bash)

# Zsh
source <(whurl completions zsh)

# Fish
whurl completions fish | source

# PowerShell
whurl completions powershell | Out-String | Invoke-Expression
```

Keep the matching command in your shell startup file and re-source it after upgrading `whurl`.

Completions include commands and options, workspace request names, environment names, and stored
run selectors. Request targets are suggested in their canonical form (for example,
`billing/list_invoices`), without `requests/` or `.hurl`; typing `requests/` continues that form
when it is useful. Workspace values are discovered from the current directory, `--workspace`,
`WHURL_WORKSPACE`, or config just as they are when running a command.

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

`?` → Config tab in the TUI edits the common settings live and writes them back, keeping your
comments; "Edit config file" opens it in `$EDITOR` and reloads it afterwards.

```toml
workspace = "~/path/to/workspace"   # used when the cwd isn't inside one

[ui]
wrap = true                       # `w` toggles
auto_switch_response = true
mouse = false                     # wheel scroll + clicks; copy then needs ⌥-drag (iTerm2/Ghostty) or shift-drag

[request]
sections = ["headers", "body", "curl"]   # display order; omit one to hide it

[response]
sections = ["headers", "body"]

[run]
default_env = "staging"
asserts = false

[viewer]
command = ""                      # `v`; empty → $VISUAL/$EDITOR (-R for vi/vim/nvim), else less
```

## TUI

`tab` moves focus between the sidebar and the main pane; `↑↓`/`jk`, `pgup`/`pgdn`, `ctrl-d/u` (half
page), `g`/`G` or `home`/`end` act on the focused one (main also: `space` page, `h/l` horizontal scroll with wrap off). Anywhere: `enter` run · `/` filter ·
`e`/`E` env · `t` asserts · `1-4` Request, Response, Meta, Source · `w` wrap · `[ ]` entries in
a chained file · `x`/`X` export Markdown/JSON to `.whurl/exports/` · `v` view the body in a viewer · `y` copy it to the clipboard · `o` open in `$EDITOR` ·
`a` new request · `d` duplicate · `m` rename · `D` delete · `C` clear history · `<`/`>`/`=` sidebar width ·
`R` reload · `?` all keys (enter runs the selected one; `/` searches keys and descriptions) and the Config tab.
The statusbar shows key hints and messages on the left; workspace, asserts, and the env on the right.

The sidebar is a folder tree that starts collapsed: `h/l` or `←→` close/open a folder (or jump to its
parent), `space`/`enter` toggle it. Filtering opens every folder holding a match. Open folders and the
last selection persist in `.whurl/state.toml`. `ui.sidebar = "flat"` shows the ranked flat list instead;
`"hidden"` gives the main pane the full width. `\` hides/shows the sidebar for the session, and
`ctrl-p` (or `/` with the sidebar hidden) opens a fuzzy picker over all requests. `<` and `>` narrow
and widen the sidebar (with the mouse on, drag its right edge); `=` fits it to the names again. The
width is not saved.

`a` (`:new <folder/name>`) creates `requests/<folder/name>.hurl` from a stub that reuses the selected
request's base URL variable, then opens it in `$EDITOR`; save it empty to discard it. `m`
(`:rename <folder/name>`) renames or moves the selected request and its run history; folders it leaves
empty are removed. `d` (`:duplicate <folder/name>`) copies the selected request and opens the copy.
These prefill the `:` bar, and `tab` completes existing folders. `D` (`:delete`, pressed twice on
the same request) deletes the selected request's file and its run history.
`C` (`:clear-history`, pressed twice) deletes every stored run for all requests and envs.

`:` opens a command bar on the statusbar line. It runs any command from `?` by name (`:expand-all`,
`:q`) plus a few that take arguments: `:env <name>`, `:tab <name>`, `:export md|json`,
`:sidebar tree|flat|hidden`, `:asserts`/`:wrap on|off`, `:cancel [all]`. `:set <key> [value]` changes a
setting for the session; `:set! <key> [value]` also saves it to the config file (with no value it saves
the current one, so `\` then `:set! ui.sidebar` keeps the sidebar). `tab` completes, `↑↓` choose, and
`enter` takes the highlighted completion when the word is unfinished.

`v` writes the current entry's body (the request body on the Request tab, else the response),
pretty-printed and already redacted, to a private temp file named for its content type, opens it in
`viewer.command`, and deletes it when the viewer exits.
`y` (or `:copy`) copies that same body to the clipboard via pbcopy, wl-copy, xclip, or xsel, else
OSC 52 (tmux needs `set -g set-clipboard on` for that).

With `ui.mouse = true` (or `:set ui.mouse on`) the wheel scrolls the pane under the pointer (the
sidebar moves the selection; overlays move theirs), and a click focuses a pane, selects a row, opens or
closes a folder, or switches tab. Mouse capture turns off the terminal's own drag-to-select, so copy
with ⌥-drag in iTerm2/Ghostty or shift-drag in most other terminals.

The tab stays put as you move between requests; when a run you started finishes and the request
is still selected, the view switches to Response. The summary shows the latency waterfall (dns, connect,
tls, send, wait, download) and a sparkline of the last 30 runs in the current env; Meta adds
per-phase bars, min/p50/p90/max, asserts, and captures.

## Secrets

Hurl redacts secrets in its terminal output but not in its JSON report, which whurl parses. whurl
masks every value from the env's secrets files as `***` before printing, storing, or exporting.
Values a response returns (a freshly minted token, say) aren't known secrets and are stored as-is
in `.whurl/runs/`.
