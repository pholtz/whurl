mod completion;
mod print;
mod tui;

use whurl_core::{config, export, run, store, workspace};

use anyhow::Result;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum, ValueHint};
use clap_complete::ArgValueCompleter;
use std::io::IsTerminal;
use std::path::PathBuf;
use workspace::Workspace;

/// whurl ("with hurl"): run the hurl requests in a workspace of `.hurl` files, see the
/// responses, and export runs for analysis.
///
/// Run with no command in a terminal to open the TUI.
#[derive(Parser)]
#[command(version)]
struct Cli {
    /// Workspace root (holds requests/ and env/). Defaults to the nearest ancestor of the
    /// current directory that has both, then `workspace` in the config file.
    #[arg(long, global = true, env = "WHURL_WORKSPACE", value_hint = ValueHint::DirPath)]
    workspace: Option<PathBuf>,

    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the interactive TUI.
    Tui {
        /// Initial filter for the request list.
        #[arg(add = ArgValueCompleter::new(completion::requests))]
        query: Option<String>,
        #[command(flatten)]
        env: EnvArg,
    },
    /// List requests in the workspace.
    #[command(visible_alias = "ls")]
    List {
        /// Filter by name (fuzzy; space-separated terms must all match).
        #[arg(add = ArgValueCompleter::new(completion::requests))]
        query: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run a request file and show the response.
    Run(RunArgs),
    /// Show a stored run (default: the latest).
    Show {
        /// `last`, `last~N`, a run id or id prefix, or a request name (its latest run).
        #[arg(default_value = "last", add = ArgValueCompleter::new(completion::runs))]
        run: String,
        #[command(flatten)]
        out: OutArgs,
    },
    /// List past runs, newest first.
    #[command(visible_alias = "log")]
    History {
        /// Only runs of requests matching this.
        #[arg(add = ArgValueCompleter::new(completion::requests))]
        query: Option<String>,
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Export a run (request, response, timings, hurl source) for agents or sharing.
    Export {
        #[arg(default_value = "last", add = ArgValueCompleter::new(completion::runs))]
        run: String,
        #[arg(short, long, value_enum, default_value_t = Format::Md)]
        format: Format,
        /// Write to this file instead of stdout.
        #[arg(short, long, value_hint = ValueHint::FilePath)]
        output: Option<PathBuf>,
    },
    /// List environments and their non-secret variables.
    Envs {
        #[arg(long)]
        json: bool,
    },
    /// Print shell-completion registration. Source its output in your shell.
    #[command(visible_alias = "completion")]
    Completions {
        #[arg(value_enum)]
        shell: completion::Shell,
    },
}

#[derive(Args)]
struct EnvArg {
    /// Environment: a file env/<ENV>.env. Default: `run.default_env` in the config, then local.
    #[arg(short, long, env = "HURL_ENV", add = ArgValueCompleter::new(completion::envs))]
    env: Option<String>,
}

#[derive(Args)]
struct RunArgs {
    /// A .hurl path or a fuzzy request name, e.g. `semantics/get_segment_size` or `seg size`.
    #[arg(add = ArgValueCompleter::new(completion::requests))]
    target: String,
    #[command(flatten)]
    env: EnvArg,
    /// Evaluate asserts (skipped by default).
    #[arg(short = 't', long)]
    test: bool,
    #[command(flatten)]
    out: OutArgs,
    /// Also write the run to this file (.json for JSON, otherwise Markdown).
    #[arg(short = 'x', long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    export: Option<PathBuf>,
    /// Extra arguments passed to hurl, after `--`.
    #[arg(last = true)]
    hurl_args: Vec<String>,
}

#[derive(Args)]
struct OutArgs {
    /// Include request and response headers.
    #[arg(short = 'i', long)]
    headers: bool,
    /// Print only the raw response body of the final request (for piping).
    #[arg(short, long, conflicts_with_all = ["json", "md"])]
    body: bool,
    /// Print the full run as JSON.
    #[arg(long, conflicts_with = "md")]
    json: bool,
    /// Print the full run as Markdown.
    #[arg(long)]
    md: bool,
    /// Don't print the response body.
    #[arg(long)]
    no_body: bool,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Md,
    Json,
}

fn main() {
    completion::handle::<Cli>();

    if let Err(e) = real_main() {
        eprintln!("whurl: {e:#}");
        std::process::exit(2);
    }
}

fn real_main() -> Result<()> {
    let Cli { workspace, cmd } = Cli::parse();
    if let Some(Cmd::Completions { shell }) = &cmd {
        return completion::print_script(*shell);
    }

    let cfg = config::Config::load()?;
    let ws = Workspace::discover(workspace, cfg.workspace())?;
    match cmd {
        None if std::io::stdout().is_terminal() => {
            let env = cfg.env(std::env::var("HURL_ENV").ok().filter(|e| !e.is_empty()));
            tui::run(ws, cfg, None, env)
        }
        None => {
            Cli::command().print_help()?;
            Ok(())
        }
        Some(Cmd::Tui { query, env }) => {
            let env = cfg.env(env.env);
            tui::run(ws, cfg, query, env)
        }
        Some(Cmd::List { query, json }) => list(&ws, &query.join(" "), json),
        Some(Cmd::Run(args)) => run_cmd(&ws, &cfg, args),
        Some(Cmd::Show { run, out }) => {
            let r = store::find(&ws, &run)?;
            output(&ws, &r, &out)
        }
        Some(Cmd::History { query, limit, json }) => history(&ws, query.as_deref(), limit, json),
        Some(Cmd::Export {
            run,
            format,
            output,
        }) => {
            let r = store::find(&ws, &run)?;
            let text = export::render(&r, format == Format::Json);
            match output {
                Some(p) => {
                    std::fs::write(&p, text)?;
                    eprintln!("{}", p.display());
                }
                None => print!("{text}"),
            }
            Ok(())
        }
        Some(Cmd::Envs { json }) => envs(&ws, json),
        Some(Cmd::Completions { .. }) => unreachable!("handled before workspace discovery"),
    }
}

fn list(ws: &Workspace, query: &str, json: bool) -> Result<()> {
    let all = ws.requests()?;
    let idx = workspace::search(&all, query);
    if json {
        let v: Vec<_> = idx
            .iter()
            .map(|&i| {
                let r = &all[i];
                serde_json::json!({
                    "name": r.rel,
                    "title": r.title,
                    "path": r.path,
                    "requests": r.entries.iter().map(|e| serde_json::json!({"method": e.method, "url": e.url, "line": e.line})).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    let p = print::Paint::stdout();
    let width = idx.iter().map(|&i| all[i].rel.len()).max().unwrap_or(0);
    for &i in &idx {
        let r = &all[i];
        let (m, url) = r
            .primary()
            .map(|e| (e.method.as_str(), e.url.as_str()))
            .unwrap_or(("?", ""));
        let more = if r.entries.len() > 1 {
            p.dim(&format!(" (+{})", r.entries.len() - 1))
        } else {
            String::new()
        };
        println!("{:<width$}  {:<6} {}{more}", r.rel, p.method(m), p.dim(url));
    }
    Ok(())
}

fn run_cmd(ws: &Workspace, cfg: &config::Config, a: RunArgs) -> Result<()> {
    let req = ws.resolve(&a.target)?;
    let env = cfg.env(a.env.env);
    ws.vars_file(&env)?;
    if !a.out.body && !a.out.json && !a.out.md && std::io::stderr().is_terminal() {
        eprintln!(
            "{}",
            print::Paint::stderr().dim(&format!("running {} [{env}]…", req.rel))
        );
    }
    let r = run::run(
        ws,
        &req,
        &run::RunOpts {
            env,
            asserts: a.test || cfg.run.asserts,
            extra_args: a.hurl_args,
        },
    )?;
    if let Some(p) = &a.export {
        let fmt = if p.extension().is_some_and(|e| e == "json") {
            Format::Json
        } else {
            Format::Md
        };
        std::fs::write(p, export::render(&r, fmt == Format::Json))?;
        eprintln!("{}", print::green(&format!("exported {}", p.display())));
    }
    output(ws, &r, &a.out)?;
    if !r.success {
        std::process::exit(r.exit_code.max(1));
    }
    Ok(())
}

fn output(_ws: &Workspace, r: &run::Run, out: &OutArgs) -> Result<()> {
    if out.json {
        print!("{}", export::render(r, true));
    } else if out.md {
        print!("{}", export::render(r, false));
    } else if out.body {
        if let Some(b) = r
            .primary()
            .and_then(|e| e.response.as_ref())
            .and_then(|r| r.body.as_ref())
        {
            print!("{b}");
        }
        if !r.stderr.is_empty() {
            eprintln!("{}", r.stderr);
        }
    } else {
        print::run(
            r,
            &print::Show {
                headers: out.headers,
                body: !out.no_body,
            },
        );
    }
    Ok(())
}

fn history(ws: &Workspace, query: Option<&str>, limit: usize, json: bool) -> Result<()> {
    let ids = match query {
        Some(q) => store::ids_for(ws, &ws.resolve(q)?.rel, None),
        None => store::ids(ws),
    };
    let runs: Vec<_> = ids
        .iter()
        .take(limit)
        .filter_map(|id| store::load(ws, id).ok())
        .collect();
    if json {
        let v: Vec<_> = runs
            .iter()
            .map(|r| {
                let e = r.primary();
                serde_json::json!({
                    "id": r.id, "request": r.request, "env": r.env, "started_at": r.started_at,
                    "success": r.success,
                    "status": e.and_then(|e| e.response.as_ref()).map(|r| r.status),
                    "latency_us": e.and_then(|e| e.total_us()),
                    "size": e.and_then(|e| e.response.as_ref()).map(|r| r.size),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        let p = print::Paint::stdout();
        for r in &runs {
            println!("{}", print::history_line(p, r));
        }
    }
    Ok(())
}

fn envs(ws: &Workspace, json: bool) -> Result<()> {
    let p = print::Paint::stdout();
    let mut all = serde_json::Map::new();
    for env in ws.envs() {
        let vars = ws.vars(&env);
        let secret_names: Vec<String> = ws
            .secrets_files(&env)
            .iter()
            .flat_map(|f| workspace::read_env(f))
            .map(|(k, _)| k)
            .collect();
        if json {
            all.insert(
                env.clone(),
                serde_json::json!({
                    "vars": vars.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect::<serde_json::Map<_, _>>(),
                    "secrets": secret_names,
                }),
            );
            continue;
        }
        println!("{}", p.bold(&env));
        for (k, v) in &vars {
            println!("  {} = {}", p.cyan(k), v);
        }
        if !secret_names.is_empty() {
            println!(
                "  {} {}",
                p.dim("secrets:"),
                p.dim(&secret_names.join(", "))
            );
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&all)?);
    }
    Ok(())
}
