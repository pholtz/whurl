# whurl

An ergonomic CLI and TUI to make http requests using [Hurl](https://hurl.dev) files.

If you are willing to structure your hurl files and environment files, then `whurl` makes it easier to invoke them. Since it delegates to hurl under the hood, this is mainly for ease of use when making requests by hand. For programmatic use cases, you should probably just continue to use `hurl` directly. I originally built this as a replacement for postman, after becoming increasingly annoyed with it. Hurl files are extremely easy to get started with, and easy to use programmatically (for example, in CI) but when you want to use the same files during development, the experience becomes a bit more clumsy. Initially, this leads to small scripts or programs to make this less verbose -- `whurl` attempts to improve that bespoke experience without rebuilding everything.

Colloquially, `whurl` stands for `with hurl`, but you're welcome to make up your own definition.

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
whurl completions # shell completions for usability
```

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

`$XDG_CONFIG_HOME/whurl/config.toml` houses your system wide hurl configuration. You can also edit your config directly from the tui with `?`.

Below is an example config file.

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

Pretty self explanatory, uses ratatui for rendering and expects a workspace just like the cli. Keybinds are discoverable via help (press `?`), or via commands (press `:`), which feature autocomplete and descriptions. You can create and edit requests, run them, and inspect timing and metadata all from a usable interface.

## GUI (experimental)

`whurl-gui` is a native window over the same workspace, built on Anomaly's cross platform desktop framework, Rune. It's an experimental implementation, so expect rough edges and breaking changes. Follows roughly a similar featureset as the tui, and uses the same workspace format. Metadata is shared between the tui and gui. Features discoverability similar to the tui via `cmd+p` / `cmd+k`, but slightly more ergonomic request editing.

```sh
cd crates/whurl-gui
cargo run --release -- --workspace ~/my-workspace -e local
cargo install --path .     # installs `whurl-gui`
```

### Crate layout

| Crate | Path | |
| --- | --- | --- |
| `whurl` | `.` | The CLI and TUI (one binary). |
| `whurl-core` | `crates/whurl-core` | Workspace, run, history, and config logic shared by every frontend. |
| `whurl-gui` | `crates/whurl-gui` | The GUI. |

`whurl-gui` is its own Cargo workspace, with its own lockfile, `target/`, and pinned toolchain
(`rust-toolchain.toml`), so building or installing the CLI never fetches Rune. Top-level `cargo`
commands build only the CLI and `whurl-core`. Run GUI commands from `crates/whurl-gui`: rustup picks
the toolchain by directory, so `--manifest-path` from the root would skip the pinned one.

## Secrets

Hurl redacts secrets in its terminal output but not in its JSON report, which whurl parses. whurl
masks every value from the env's secrets files as `***` before printing, storing, or exporting.
Values a response returns (a freshly minted token, say) aren't known secrets and are stored as-is
in `.whurl/runs/`.
