//! Drawing the state: sidebar, request header, tabs, and the palette and dialogs over them.

use crate::app::{Confirm, Naming, PaletteMode, SIDEBAR_W, State, TABS, Tab, Var};
use crate::commands::{self, COMMANDS};
use rune::{
    Color, Cursor, Cx, Drag, El, Key, KeyEvent, Phase, ScrollSpec, Span, TextEdit, TextStyle, Tx,
    col, el, oklch, row, text,
};
use rune_kit::syntax::{Paint, Syntax, SyntaxTheme};
use rune_kit::{
    Button, ButtonVariant, Checkbox, Dialog, FileNode, Kit, Menu, MenuItem, PaletteSeed,
    SegmentedControl, Select, SelectAppearance, Spinner, Switch, TextInput, Tooltip, Tree,
    TreeItem,
};
use std::rc::Rc;
use whurl_core::body::{self, Side};
use whurl_core::config::{REQUEST_SECTIONS, RESPONSE_SECTIONS};
use whurl_core::export::{self, ms};
use whurl_core::run::{Entry, Header, Run};
use whurl_core::settings::THEMES;
use whurl_core::workspace;
use whurl_core::workspace::METHODS;

pub fn view(s: &State, cx: &mut Cx<State>) -> El {
    let k = kit(&s.cfg.ui.theme, cx.system_dark());
    if let Some(e) = &s.fatal {
        let msg = col()
            .padding(24.0)
            .gap(8.0)
            .child(
                text("whurl-gui couldn't open a workspace")
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base),
            )
            .child(
                text(e.clone())
                    .font(k.ty.mono)
                    .color(k.c.danger.icon)
                    .selectable(true)
                    .wrap(),
            );
        return col().w_full().h_full().fill(k.c.page).child(msg);
    }
    let mut layer = vec![];
    // macOS draws its window buttons over the top-left corner: keep content clear of them.
    let inset = cx.titlebar_inset();
    let mut panes = row().w_full().grow(1.0).min_h(0.0);
    if s.sidebar {
        panes = panes
            .child(sidebar(s, &k, inset, &mut layer))
            .child(grip(&k));
    }
    let lead = if s.sidebar || inset == 0.0 {
        0.0
    } else {
        inset
    };
    panes = panes.child(content(s, &k, lead, &mut layer));
    let base = col()
        .w_full()
        .h_full()
        .fill(k.c.page)
        .child(panes)
        .child(status_bar(s, &k));
    palette(s, &k, &mut layer);
    naming(s, &k, &mut layer);
    confirm(s, &k, &mut layer);
    settings(s, &k, &mut layer);
    env_dialog(s, &k, &mut layer);
    rune_kit::root(base, layer)
}

// --- Themes ---------------------------------------------------------------------------------

/// An sRGB `0xRRGGBB` as a colour.
fn rgb(hex: u32) -> Color {
    let ch = |shift: u32| {
        let c = ((hex >> shift) & 0xff) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Color {
        r: ch(16),
        g: ch(8),
        b: ch(0),
        a: 1.0,
    }
}

/// The kit for `ui.theme`: `default` follows the system's light or dark.
fn kit(theme: &str, system_dark: bool) -> Kit {
    match theme {
        "light" => Kit::neutral(false),
        "dark" => Kit::neutral(true),
        "nord" => Kit::from_palette(PaletteSeed {
            bg: rgb(0x2e3440),
            fg: rgb(0xeceff4),
            accent: rgb(0x88c0d0),
        }),
        _ => Kit::neutral(system_dark),
    }
}

/// Token colours: Nord's own for nord, else the kit's.
fn syntax(s: &State, k: &Kit) -> SyntaxTheme {
    let t = SyntaxTheme::from_kit(k);
    if s.cfg.ui.theme != "nord" {
        return t;
    }
    t.with(Syntax::Keyword, rgb(0x81a1c1))
        .with(Syntax::String, rgb(0xa3be8c))
        .with(Syntax::Number, rgb(0xb48ead))
        .with(Syntax::Type, rgb(0x8fbcbb))
        .with(Syntax::Function, rgb(0x88c0d0))
        .with(Syntax::Property, rgb(0x88c0d0))
        .with(Syntax::Attribute, rgb(0xd08770))
        .with(Syntax::Link, rgb(0x5e81ac))
        .with(Syntax::Comment, rgb(0x7b88a1))
}

// --- Sidebar ---------------------------------------------------------------------------------

fn sidebar(s: &State, k: &Kit, inset: f32, layer: &mut Vec<El>) -> El {
    let env = Select::new("env", &s.envs)
        .current(Some(&s.env))
        .appearance(SelectAppearance::Field)
        .width(s.sidebar_w - 16.0)
        .open(s.env_open)
        .on_toggle(|s: &mut State| s.env_open = !s.env_open)
        .on_close(|s: &mut State| s.env_open = false)
        .on_choose(|s: &mut State, v: &str, _: &mut Tx| s.set_env(v))
        .build(k, layer);
    let filter = TextInput::new(&s.filter)
        .width(None)
        .placeholder("Filter requests  ⌘F")
        .leading_icon(k.glyph.search)
        .clear_button()
        .on_edit(|s: &mut State| &mut s.filter)
        .on_change(|s: &mut State, _: &mut Tx| s.apply_filter())
        .on_key(|s: &mut State, ev: &KeyEvent, tx: &mut Tx| match ev.key {
            Key::Escape => {
                s.filter.clear();
                s.apply_filter();
                tx.blur();
                true
            }
            _ => false,
        })
        .on_submit(|s: &mut State, tx: &mut Tx| {
            // Enter picks the first match.
            let q = s.filter.text.clone();
            if let Some(&i) = workspace::search(&s.requests, &q).first() {
                let rel = s.requests[i].rel.clone();
                s.select(&rel);
            }
            tx.blur();
        })
        .build(k);
    let running: Vec<String> = s
        .running
        .keys()
        .filter(|k| k.1 == s.env)
        .map(|k| k.0.clone())
        .collect();
    let (c, small) = (k.c, k.ty.text_xsmall_medium);
    let requests = Tree::new(
        |s: &State| &s.files,
        |s: &mut State| &mut s.files,
        |n: &FileNode<usize>| &n.name,
    )
    .name("Requests")
    .row(move |n, r| {
        let item = TreeItem::file(n, r);
        // Folders in the base colour with an accent icon; requests a step quieter.
        if n.dir {
            return item.label_color(c.text_base).icon_color(c.icon_accent);
        }
        let item = item
            .label_color(if r.selected {
                c.text_base
            } else {
                c.text_muted
            })
            .icon_color(c.icon_faint);
        if running.contains(&n.path) {
            return item.trailing(text("●").key("running").font(small).color(c.warning.icon));
        }
        item
    })
    .on_select(|s: &mut State, _: &str, _: &mut Tx| s.selected_changed())
    .on_open(|s: &mut State, _: &str, _: &mut Tx| s.run_current())
    .build(s, k)
    .grow(1.0)
    .min_h(0.0);
    let mut out = col()
        .key("sidebar")
        .w(s.sidebar_w)
        .h_full()
        .padding(8.0)
        .gap(8.0)
        .fill(k.c.bg_base);
    if inset > 0.0 {
        out = out
            .pt(0.0)
            .child(el().key("titlebar").w_full().h(32.0).window_drag());
    }
    out.child(env).child(filter).child(requests)
}

/// The sidebar's edge: drag to resize, double-click for the default width.
fn grip(k: &Kit) -> El {
    row()
        .key("sidebar-grip")
        .w(6.0)
        .h_full()
        .fill(k.c.bg_base)
        .child(el().w(1.0).h_full().fill(k.c.border_base))
        .cursor(Cursor::ResizeColumn)
        .on_drag(|s: &mut State, d: &mut Drag, tx: &mut Tx| {
            tx.instant();
            match d.phase {
                Phase::Down => {
                    d.data = s.sidebar_w;
                    s.resizing = true;
                }
                Phase::Move => s.sidebar_w = (d.data + d.dx).clamp(180.0, 640.0).round(),
                Phase::Up => {
                    if d.cancelled() {
                        s.sidebar_w = d.data;
                    }
                    s.resizing = false;
                }
            }
        })
        .on_double_press(|s: &mut State| s.sidebar_w = SIDEBAR_W)
}

// --- Main pane -------------------------------------------------------------------------------

fn content(s: &State, k: &Kit, lead: f32, layer: &mut Vec<El>) -> El {
    let Some(req) = s.current() else {
        let hint = text("Select a request, or ⌘P to find one and ⌘K for commands.")
            .font(k.ty.ui)
            .color(k.c.text_muted);
        return col()
            .key("main")
            .grow(1.0)
            .h_full()
            .padding(24.0)
            .child(hint);
    };
    let key = (req.rel.clone(), s.env.clone());
    let run = s.run();
    let running = s.running.contains_key(&key);

    // Name, title and save state.
    let mut name = row().gap(10.0).center_y().child(
        text(req.rel.clone())
            .font(k.ty.ui_medium)
            .color(k.c.text_base),
    );
    if let Some(t) = &req.title
        && Some(t.as_str()) != req.rel.rsplit('/').next()
    {
        name = name.child(text(t.clone()).font(k.ty.small).color(k.c.text_faint));
    }
    name = name.child(el().grow(1.0));
    if s.dirty() {
        name = name
            .child(text("● unsaved").font(k.ty.small).color(k.c.warning.icon))
            .child(
                Button::new("Revert")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| s.revert())
                    .build(k),
            )
            .child(
                Button::new("Save")
                    .shortcut("⌘S")
                    .on_press(|s: &mut State| {
                        if s.save() {
                            s.say("saved", false);
                        }
                    })
                    .build(k),
            );
    }

    // Method and URL as fields, then Run and the ⋯ menu.
    let action = if running {
        row()
            .gap(8.0)
            .center_y()
            .child(Spinner::new().size(14.0).build(k))
            .child(
                Button::new("Cancel")
                    .shortcut("⌘.")
                    .on_press(|s: &mut State| s.cancel_current())
                    .build(k),
            )
    } else {
        row().child(
            Button::new("Run")
                .variant(ButtonVariant::Submit)
                .shortcut("⌘↩")
                .on_press(|s: &mut State| s.run_current())
                .build(k),
        )
    };
    let mut line = row().gap(8.0).center_y();
    if s.form {
        let mut methods: Vec<String> = METHODS.iter().map(|m| m.to_string()).collect();
        if !methods.contains(&s.method) {
            methods.insert(0, s.method.clone());
        }
        let method = Select::new("method", &methods)
            .current(Some(&s.method))
            .appearance(SelectAppearance::Field)
            .width(112.0)
            .open(s.method_open)
            .on_toggle(|s: &mut State| s.method_open = !s.method_open)
            .on_close(|s: &mut State| s.method_open = false)
            .on_choose(|s: &mut State, v: &str, _: &mut Tx| s.set_method(v))
            .build(k, layer);
        let spans: Vec<Span> = crate::hurl::var_ranges(&s.url.text)
            .into_iter()
            .map(|(r, name)| var_span(s, k, r, name))
            .collect();
        let url = TextInput::new(&s.url)
            .spans(spans)
            .width(None)
            .text_style(k.ty.mono)
            .placeholder("{{host}}/path")
            .on_edit(|s: &mut State| &mut s.url)
            .on_change(|s: &mut State, _: &mut Tx| s.form_changed())
            .on_submit(|s: &mut State, tx: &mut Tx| {
                tx.blur();
                s.run_current();
            })
            .build(k)
            .key("url")
            .grow(1.0)
            .min_w(0.0);
        line = line.child(method).child(url);
    } else {
        line = line.child(
            text("No request line yet: write one in the Source tab.")
                .font(k.ty.ui)
                .color(k.c.text_muted)
                .grow(1.0),
        );
    }
    line = line.child(action).child(actions_menu(s, k, layer));

    let mut header = col().gap(10.0).child(name).child(line);
    // The request's variables, then the URL as it will be sent when the env changes it.
    if s.form {
        let mut under = row().gap(12.0).center_y().pl(120.0);
        let chips = var_chips(s, k, layer);
        let any = chips.is_some();
        if let Some(c) = chips {
            under = under.child(c);
        }
        let sent = s.ws.interpolate(&s.url.text, &s.vars);
        let changed = sent != s.url.text;
        if changed {
            under = under.child(
                text(format!("→ {sent}"))
                    .font(k.ty.small)
                    .color(k.c.text_faint)
                    .selectable(true)
                    .grow(1.0)
                    .min_w(0.0),
            );
        }
        if any || changed {
            header = header.child(under);
        }
    }
    let header = header
        .key("header")
        .w_full()
        .px(20.0)
        .pl(20.0 + lead)
        .py(16.0)
        .fill(k.c.bg_base)
        .window_drag();

    let labels: Vec<&str> = TABS.iter().map(|t| t.1).collect();
    let at = TABS.iter().position(|t| t.0 == s.tab);
    let tabs = SegmentedControl::new(&labels, at)
        .fit()
        .on_pick(|s: &mut State, i: usize, _: &mut Tx| s.tab = TABS[i].0)
        .build(k);
    let mut bar = row().gap(12.0).center_y().child(tabs);
    if let Some(r) = run {
        bar = bar.child(run_line(r, s.entry_index().unwrap_or(0), k));
    }

    let mut out = col()
        .key("main-body")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .px(20.0)
        .pt(14.0)
        .pb(12.0)
        .gap(12.0)
        .child(bar);
    if let (Some(r), Some(i)) = (run, s.entry_index())
        && r.entries.len() > 1
        && matches!(s.tab, Tab::Response | Tab::Meta)
    {
        out = out.child(entry_switcher(r, i, k));
    }

    let body = match s.tab {
        Tab::Source => editor(
            s,
            k,
            "source",
            &s.draft,
            Some("hurl"),
            |s: &mut State| &mut s.draft,
            Some(State::sync_form),
        ),
        Tab::Meta => meta(s, k),
        Tab::Request => request_tab(s, k, run),
        Tab::Response => match (run, s.entry_index()) {
            (Some(r), Some(i)) => entry(s, k, r, &r.entries[i], Tab::Response),
            _ => {
                let msg = if running {
                    "Running…"
                } else {
                    "No runs yet in this env. Run it with ⌘↩."
                };
                text(msg).font(k.ty.ui).color(k.c.text_muted)
            }
        },
    };
    let pane = col()
        .key(format!("body-{:?}", s.tab))
        .grow(1.0)
        .min_h(0.0)
        .child(body);
    let pane = if s.wrap || s.tab == Tab::Meta {
        pane.scroll(ScrollSpec {
            stick_bottom: false,
        })
    } else {
        pane.scroll_xy()
    };
    col()
        .key("main")
        .grow(1.0)
        .min_w(0.0)
        .h_full()
        .child(header)
        .child(out.child(pane))
}

/// The Request tab: the file's headers and body as editors, then what the last run sent.
fn request_tab(s: &State, k: &Kit, run: Option<&Run>) -> El {
    let mut out = col().gap(10.0);
    if s.form {
        out = out
            .child(section("HEADERS", k))
            .child(editor(
                s,
                k,
                "form-headers",
                &s.headers,
                Some("http"),
                |s: &mut State| &mut s.headers,
                Some(State::form_changed),
            ))
            .child(section("BODY", k))
            .child(editor(
                s,
                k,
                "form-body",
                &s.body,
                Some("hurl"),
                |s: &mut State| &mut s.body,
                Some(State::form_changed),
            ))
            .child(
                text("Edits go to the .hurl file: ⌘S saves, and running saves first. The Source tab has the whole file.")
                    .font(k.ty.small)
                    .color(k.c.text_faint)
                    .wrap(),
            );
    }
    if let (Some(r), Some(i)) = (run, s.entry_index()) {
        let mut sent = col().gap(10.0).pt(12.0).child(section("LAST SENT", k));
        if r.entries.len() > 1 {
            sent = sent.child(entry_switcher(r, i, k));
        }
        out = out.child(sent.child(entry(s, k, r, &r.entries[i], Tab::Request)));
    }
    out
}

/// The ⋯ menu: what can be done with the selected request.
fn actions_menu(s: &State, k: &Kit, layer: &mut Vec<El>) -> El {
    let mut menu = Menu::new("actions")
        .trigger(
            Button::new("⋯")
                .variant(ButtonVariant::Ghost)
                .expanded(s.menu_open)
                .on_press(|s: &mut State| s.menu_open = !s.menu_open)
                .build(k),
        )
        .open(s.menu_open)
        .on_close(|s: &mut State| s.menu_open = false)
        .min_width(240.0);
    let groups: [&[&str]; 3] = [
        &["edit", "reveal", "rename", "duplicate", "delete"],
        &["export-md", "export-json"],
        &["copy", "copy-curl", "copy-run"],
    ];
    for (g, ids) in groups.iter().enumerate() {
        if g > 0 {
            menu = menu.separator();
        }
        for id in *ids {
            let Some(c) = COMMANDS.iter().find(|c| c.id == *id) else {
                continue;
            };
            let run = c.run;
            let mut item = MenuItem::new(c.title).danger(*id == "delete").on_select_tx(
                move |s: &mut State, tx: &mut Tx| {
                    s.menu_open = false;
                    run(s, tx)
                },
            );
            if let Some(l) = commands::label(c) {
                item = item.shortcut(l);
            }
            menu = menu.item(item);
        }
    }
    menu.build(k, layer)
}

/// `200 OK · 41ms · 1.2 KB · local · 2026-11-23 10:04`, for the shown entry.
fn run_line(r: &Run, i: usize, k: &Kit) -> El {
    let e = &r.entries[i.min(r.entries.len().saturating_sub(1))];
    let mut parts = vec![];
    let color = match &e.response {
        Some(resp) => {
            parts.push(format!("{} {}", resp.status, export::reason(resp.status)));
            if let Some(us) = e.total_us() {
                parts.push(ms(us));
            }
            parts.push(export::bytes(resp.size));
            status_color(resp.status, k)
        }
        None => {
            parts.push("no response".into());
            k.c.danger.icon
        }
    };
    let failed = e.failed_asserts().count();
    if failed > 0 {
        parts.push(format!(
            "{failed} assert{} failed",
            if failed == 1 { "" } else { "s" }
        ));
    }
    parts.push(r.env.clone());
    parts.push(
        r.started_at
            .chars()
            .take(16)
            .collect::<String>()
            .replace('T', " "),
    );
    let color = if failed > 0 { k.c.danger.icon } else { color };
    text(parts.join(" · ")).font(k.ty.small).color(color)
}

fn status_color(status: u16, k: &Kit) -> Color {
    match status {
        200..300 => k.c.success.icon,
        300..400 => k.c.warning.icon,
        _ => k.c.danger.icon,
    }
}

fn entry_switcher(r: &Run, i: usize, k: &Kit) -> El {
    let e = &r.entries[i];
    row()
        .gap(8.0)
        .center_y()
        .child(
            Button::new("‹")
                .variant(ButtonVariant::Ghost)
                .shortcut("⌘[")
                .on_press(|s: &mut State| s.cycle_entry(-1))
                .build(k),
        )
        .child(
            text(format!("entry {}/{}", i + 1, r.entries.len()))
                .font(k.ty.small)
                .color(k.c.text_base),
        )
        .child(
            Button::new("›")
                .variant(ButtonVariant::Ghost)
                .shortcut("⌘]")
                .on_press(|s: &mut State| s.cycle_entry(1))
                .build(k),
        )
        .child(
            text(format!("line {} · {} {}", e.line, e.method, e.url))
                .font(k.ty.mono)
                .color(k.c.text_muted),
        )
}

fn headers_el(hs: &[Header], k: &Kit) -> El {
    let lines: Vec<String> = hs
        .iter()
        .map(|h| format!("{}: {}", h.name, h.value))
        .collect();
    text(lines.join("\n"))
        .font(k.ty.mono)
        .color(k.c.text_muted)
        .selectable(true)
}

fn section(title: &str, k: &Kit) -> El {
    text(title.to_owned())
        .font(k.ty.text_xsmall_medium)
        .color(k.c.text_faint)
}

/// Request or response: the request line, headers, body (and curl), per the config's sections.
fn entry(s: &State, k: &Kit, r: &Run, e: &Entry, tab: Tab) -> El {
    let mut out = col().gap(12.0);
    let (side, headers, sections) = match tab {
        Tab::Request => (
            Side::Request,
            &e.request.headers,
            s.cfg.request.or(REQUEST_SECTIONS),
        ),
        _ => match &e.response {
            Some(resp) => (
                Side::Response,
                &resp.headers,
                s.cfg.response.or(RESPONSE_SECTIONS),
            ),
            None => {
                let err = if r.stderr.is_empty() {
                    "no response".to_string()
                } else {
                    r.stderr.clone()
                };
                return text(err)
                    .font(k.ty.mono)
                    .color(k.c.danger.icon)
                    .selectable(true)
                    .wrap();
            }
        },
    };
    if side == Side::Request {
        out = out.child(
            text(format!("{} {}", e.method, e.url))
                .font(k.ty.mono)
                .color(k.c.text_base)
                .selectable(true),
        );
    }
    for sec in sections {
        match sec {
            "headers" if !headers.is_empty() => {
                let lines: Vec<String> = headers
                    .iter()
                    .map(|h| format!("{}: {}", h.name, h.value))
                    .collect();
                out = out.child(scratch(
                    s,
                    k,
                    format!("{}:{}:{:?}:headers", r.id, e.index, side),
                    &lines.join("\n"),
                    Some("http"),
                ))
            }
            "body" => match body::of(e, side) {
                Ok(b) => {
                    out = out.child(scratch(
                        s,
                        k,
                        format!("{}:{}:{:?}", r.id, e.index, side),
                        &b.text,
                        Some(b.ext),
                    ));
                    if b.truncated {
                        out = out.child(
                            text("(truncated: export the run for all of it)")
                                .font(k.ty.small)
                                .color(k.c.text_faint),
                        );
                    }
                }
                Err(why) => out = out.child(text(why).font(k.ty.small).color(k.c.text_faint)),
            },
            "curl" if !e.curl.is_empty() => {
                out = out.child(section("CURL", k)).child(scratch(
                    s,
                    k,
                    format!("{}:{}:curl", r.id, e.index),
                    &e.curl,
                    Some("sh"),
                ))
            }
            _ => {}
        }
    }
    if side == Side::Response && !r.stderr.is_empty() && !r.success {
        out = out.child(
            text(r.stderr.clone())
                .font(k.ty.mono)
                .color(k.c.danger.icon)
                .selectable(true)
                .wrap(),
        );
    }
    out
}

/// Highlight spans for `src`, cached under `id` until it changes. In hurl source, each
/// `{{var}}` takes the accent colour when this env gives it a value and the warning one when not.
fn highlight(s: &State, k: &Kit, id: &str, src: &str, lang: Option<&str>) -> Vec<Span> {
    let mut cache = s.cache.borrow_mut();
    if !cache.get(id).is_some_and(|(t, _)| **t == *src) {
        let hl = match lang {
            Some("hurl" | "http") => crate::hurl::highlight(src).into(),
            Some("dotenv") => crate::hurl::highlight_env(src).into(),
            _ => rune_kit::syntax::highlight(lang, src),
        };
        cache.insert(id.to_owned(), (Rc::from(src), hl));
    }
    let hl = &cache[id].1;
    let mut spans = syntax(s, k).spans(hl, 0);
    if matches!(lang, Some("hurl" | "http")) {
        for ((r, p), sp) in hl.iter().zip(spans.iter_mut()) {
            if *p == Paint::Kind(Syntax::Variable) {
                let name = src[r.clone()].trim_matches(['{', '}']).trim();
                *sp = var_span(s, k, r.clone(), name);
            }
        }
    }
    spans
}

/// The span for a `{{var}}` at `r`, coloured by whether it resolves.
fn var_span(s: &State, k: &Kit, r: std::ops::Range<usize>, name: &str) -> Span {
    let mut sp = Span::new(r.start, r.end);
    if s.var(name).resolved() {
        sp.color = Some(k.c.text_accent);
        sp.bg = Some(k.alpha(k.c.text_accent, 0.12));
        sp.bg_pad = 1.0;
    } else {
        sp.color = Some(k.c.warning.icon);
        sp.underline = Some(k.c.warning.icon);
    }
    sp
}

/// The request's `{{vars}}` as chips; hovering one shows where its value comes from.
fn var_chips(s: &State, k: &Kit, layer: &mut Vec<El>) -> Option<El> {
    let src = format!("{}\n{}\n{}", s.url.text, s.headers.text, s.body.text);
    let names = crate::hurl::var_names(&src);
    if names.is_empty() {
        return None;
    }
    let mut out = row().gap(6.0).center_y();
    for name in names {
        let v = s.var(&name);
        let (ink, fill) = if v.resolved() {
            (k.c.text_accent, k.alpha(k.c.text_accent, 0.12))
        } else {
            (k.c.warning.icon, k.alpha(k.c.warning.icon, 0.14))
        };
        let tip = match v {
            Var::Env("") => format!("{name} is empty in {}", s.env),
            Var::Env(v) => clip(v, 240),
            Var::Secret => format!("{name} is a secret in {} (hidden)", s.env),
            Var::Capture => format!("{name} is captured by an earlier entry when it runs"),
            Var::Builtin => format!("{name} is generated by hurl on each run"),
            Var::Missing => format!("{name} isn't set in {}", s.env),
        };
        let key = format!("var-{name}");
        let at = name.clone();
        let chip = row()
            .key(key.clone())
            .px(6.0)
            .py(2.0)
            .radius(k.radius.sm)
            .fill(fill)
            .child(
                text(name.clone())
                    .font(TextStyle {
                        size: k.ty.small.size,
                        line_height: 0.0,
                        ..k.ty.mono
                    })
                    .color(ink),
            )
            .on_hover_change(move |s: &mut State, inside, _: &mut Tx| {
                if inside {
                    s.hover_var = Some(at.clone());
                } else if s.hover_var.as_deref() == Some(at.as_str()) {
                    s.hover_var = None;
                }
            });
        let open = s.hover_var.as_deref() == Some(name.as_str());
        out = out.child(
            Tooltip::new(key)
                .trigger(chip)
                .text(k, tip)
                .open(open)
                .build(k, layer),
        );
    }
    Some(out)
}

/// `s` cut to `n` characters, with an ellipsis when it was longer.
fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// A highlighted, multi-line monospace editor over `edit`; `changed` runs after each edit.
fn editor<F>(
    s: &State,
    k: &Kit,
    id: &str,
    edit: &TextEdit,
    lang: Option<&str>,
    access: F,
    changed: Option<fn(&mut State)>,
) -> El
where
    F: for<'a> Fn(&'a mut State) -> &'a mut TextEdit + Clone + 'static,
{
    let spans = highlight(s, k, id, &edit.text, lang);
    let newline = access.clone();
    let mut field = rune::text_input(edit, k.ty.mono)
        .key(id.to_owned())
        .w_full()
        .min_h(40.0)
        .color(k.c.text_base)
        .caret(k.c.text_base, k.selection())
        .spans(spans)
        .on_edit(access)
        // Return is a new line here, not submit.
        .on_submit(move |s: &mut State, _: &mut Tx| {
            newline(s).insert("\n");
            if let Some(f) = changed {
                f(s);
            }
        });
    if let Some(f) = changed {
        field = field.on_input_change(move |s: &mut State, _: &mut Tx| f(s));
    }
    let border = if edit.focused {
        k.c.border_focus
    } else {
        k.c.border_base
    };
    col()
        .key(format!("{id}-box"))
        .w_full()
        .px(12.0)
        .py(10.0)
        .radius(k.radius.md)
        .fill(k.c.bg_base)
        .border(1.0, border)
        .child(field)
}

/// A run's text in an editor: selectable, and editable as scratch that isn't saved anywhere.
fn scratch(s: &State, k: &Kit, id: String, src: &str, lang: Option<&str>) -> El {
    let edit = s
        .scratch
        .borrow_mut()
        .entry(id.clone())
        .or_insert_with(|| TextEdit::new(src))
        .clone();
    let at = id.clone();
    editor(
        s,
        k,
        &id,
        &edit,
        lang,
        move |s: &mut State| {
            s.scratch
                .get_mut()
                .entry(at.clone())
                .or_insert_with(|| TextEdit::new(""))
        },
        None,
    )
}

// --- Meta tab --------------------------------------------------------------------------------

const PHASE_HUES: [f32; 6] = [300.0, 250.0, 200.0, 150.0, 70.0, 30.0];

fn meta(s: &State, k: &Kit) -> El {
    let mut out = col().gap(16.0).max_w(760.0);
    let key = s.key();
    if let (Some(r), Some(i)) = (s.run(), s.entry_index()) {
        let e = &r.entries[i];
        let mut info = vec![
            format!("id       {}", r.id),
            format!("started  {}", r.started_at),
            format!("env      {}", r.env),
        ];
        if r.entries.len() > 1 {
            info.push(format!(
                "entry    {}/{} (line {})",
                i + 1,
                r.entries.len(),
                e.line
            ));
        }
        if let Some(resp) = &e.response {
            info.push(format!("http     {}", resp.http_version));
        }
        info.push(format!(
            "hurl     exit {} · {}ms wall time (includes startup)",
            r.exit_code, r.wall_ms
        ));
        out = out.child(
            col().gap(6.0).child(section("RUN", k)).child(
                text(info.join("\n"))
                    .font(k.ty.mono)
                    .color(k.c.text_muted)
                    .selectable(true),
            ),
        );

        if let Some(t) = &e.timings {
            out = out.child(col().gap(6.0).child(section("TIMING", k)).child(waterfall(
                t,
                k,
                s.dark(k),
            )));
        }

        let failed: Vec<_> = e.failed_asserts().collect();
        if !e.asserts.is_empty() {
            let title = format!(
                "ASSERTS {}/{} PASSED",
                e.asserts.len() - failed.len(),
                e.asserts.len()
            );
            let mut sec = col().gap(6.0).child(section(&title, k));
            for a in failed {
                sec = sec.child(
                    text(a.message.clone().unwrap_or_else(|| "failed".into()))
                        .font(k.ty.mono)
                        .color(k.c.danger.icon)
                        .selectable(true)
                        .wrap(),
                );
            }
            out = out.child(sec);
        } else if !r.asserts {
            out = out.child(
                text("Asserts were off for this run (⌘T turns them on).")
                    .font(k.ty.small)
                    .color(k.c.text_faint),
            );
        }
        if !e.captures.is_empty() {
            out = out.child(
                col()
                    .gap(6.0)
                    .child(section("CAPTURES", k))
                    .child(headers_el(&e.captures, k)),
            );
        }
    }

    if let Some(lat) = key.as_ref().and_then(|k| s.latencies.get(k))
        && lat.len() > 1
    {
        let mut sorted = lat.clone();
        sorted.sort_unstable();
        let pct = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        let line = format!(
            "min {}  p50 {}  p90 {}  max {}",
            ms(sorted[0]),
            ms(pct(0.5)),
            ms(pct(0.9)),
            ms(*sorted.last().unwrap())
        );
        out = out.child(
            col()
                .gap(6.0)
                .child(section(
                    &format!("LAST {} RUNS IN {}", lat.len(), s.env.to_uppercase()),
                    k,
                ))
                .child(sparkline(lat, k))
                .child(text(line).font(k.ty.mono).color(k.c.text_muted)),
        );
    }

    if let Some(past) = key.as_ref().and_then(|k| s.history.get(k))
        && !past.is_empty()
    {
        let shown = s.run().map(|r| r.id.clone());
        let mut list = col().gap(2.0).child(section("HISTORY", k));
        for p in past {
            let id = p.id.clone();
            let on = shown.as_deref() == Some(&p.id);
            let color = if p.ok {
                k.c.text_muted
            } else {
                k.c.danger.icon
            };
            list = list.child(
                row()
                    .key(format!("past-{}", p.id))
                    .px(6.0)
                    .py(3.0)
                    .radius(4.0)
                    .fill(if on { k.c.bg_base } else { Color::TRANSPARENT })
                    .child(text(p.line.clone()).font(k.ty.mono).color(if on {
                        k.c.text_base
                    } else {
                        color
                    }))
                    .on_press(move |s: &mut State| s.show_run(&id)),
            );
        }
        out = out.child(list);
    }
    if s.run().is_none() {
        out = out.child(
            text("No runs yet in this env.")
                .font(k.ty.ui)
                .color(k.c.text_muted),
        );
    }
    out
}

impl State {
    fn dark(&self, k: &Kit) -> bool {
        k.dark
    }
}

fn phase_color(i: usize, dark: bool) -> Color {
    oklch(
        if dark { 0.72 } else { 0.6 },
        0.14,
        PHASE_HUES[i % PHASE_HUES.len()],
    )
}

/// Each phase as a bar starting where the previous one ended.
fn waterfall(t: &whurl_core::run::Timings, k: &Kit, dark: bool) -> El {
    const W: f32 = 420.0;
    let total = t.total.max(1) as f32;
    let mut out = col().gap(4.0);
    let mut offset = 0u64;
    for (i, p) in t.phases().iter().enumerate() {
        let start = offset as f32 / total * W;
        let w = if p.us == 0 {
            0.0
        } else {
            (p.us as f32 / total * W).max(2.0).min(W - start)
        };
        offset += p.us;
        let track = row()
            .w(W)
            .h(10.0)
            .radius(3.0)
            .fill(k.c.bg_base)
            .child(el().w(start))
            .child(el().w(w).h_full().radius(3.0).fill(phase_color(i, dark)));
        out = out.child(
            row()
                .gap(10.0)
                .center_y()
                .child(
                    text(p.name)
                        .font(k.ty.small)
                        .color(phase_color(i, dark))
                        .w(70.0),
                )
                .child(track)
                .child(text(ms(p.us)).font(k.ty.mono).color(k.c.text_muted)),
        );
    }
    out.child(
        row()
            .gap(10.0)
            .center_y()
            .child(
                text("total")
                    .font(k.ty.ui_medium)
                    .color(k.c.text_base)
                    .w(70.0),
            )
            .child(el().w(W))
            .child(text(ms(t.total)).font(k.ty.mono).color(k.c.text_base)),
    )
}

/// Latencies oldest to newest as bars.
fn sparkline(lat: &[u64], k: &Kit) -> El {
    const H: f32 = 36.0;
    let max = lat.iter().copied().max().unwrap_or(1).max(1) as f32;
    let mut out = row().h(H).gap(2.0).items_end();
    for &us in lat.iter().rev() {
        out = out.child(
            el().w(6.0)
                .h((us as f32 / max * H).max(2.0))
                .radius(1.0)
                .fill(k.c.warning.icon),
        );
    }
    out
}

// --- Status bar ------------------------------------------------------------------------------

fn status_bar(s: &State, k: &Kit) -> El {
    let mut left = row().grow(1.0).min_w(0.0).gap(8.0).center_y();
    if let Some((msg, err)) = &s.flash {
        let color = if *err {
            k.c.danger.icon
        } else {
            k.c.text_muted
        };
        left = left.child(
            text(msg.clone())
                .font(k.ty.small)
                .color(color)
                .selectable(true),
        );
    }
    let mut right = row().gap(12.0).center_y();
    if !s.running.is_empty() {
        right = right.child(
            text(format!("{} running", s.running.len()))
                .font(k.ty.small)
                .color(k.c.warning.icon),
        );
    }
    if s.asserts {
        right = right.child(text("asserts on").font(k.ty.small).color(k.c.text_muted));
    }
    right = right.child(
        text("⌘K commands · ⌘P go to")
            .font(k.ty.small)
            .color(k.c.text_faint),
    );
    row()
        .key("status")
        .w_full()
        .h(28.0)
        .px(12.0)
        .center_y()
        .fill(k.c.bg_base)
        .child(left)
        .child(right)
}

// --- Palette ---------------------------------------------------------------------------------

#[derive(Clone)]
pub enum Item {
    Command(usize),
    Env(String),
    Theme(&'static str),
    Request(usize),
}

/// What the palette lists for its query, best first.
pub fn items(s: &State) -> Vec<Item> {
    let Some(p) = &s.palette else { return vec![] };
    let q = s.query.text.as_str();
    match p.mode {
        PaletteMode::Requests => workspace::search(&s.requests, q)
            .into_iter()
            .take(200)
            .map(Item::Request)
            .collect(),
        PaletteMode::Commands => {
            let mut scored: Vec<(i64, usize, Item)> = COMMANDS
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    commands::score(q, &format!("{} {}", c.title, c.id))
                        .map(|sc| (sc, i, Item::Command(i)))
                })
                .collect();
            for (i, e) in s.envs.iter().enumerate() {
                if let Some(sc) = commands::score(q, &format!("env {e}")) {
                    scored.push((sc, COMMANDS.len() + i, Item::Env(e.clone())));
                }
            }
            for (i, t) in THEMES.iter().enumerate() {
                if let Some(sc) = commands::score(q, &format!("theme {t}")) {
                    let at = COMMANDS.len() + s.envs.len() + i;
                    scored.push((sc, at, Item::Theme(t)));
                }
            }
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            scored.into_iter().map(|t| t.2).collect()
        }
    }
}

pub fn activate(s: &mut State, item: Item, tx: &mut Tx) {
    s.palette = None;
    s.query.focused = false;
    match item {
        Item::Command(i) => (COMMANDS[i].run)(s, tx),
        Item::Env(e) => s.set_env(&e),
        Item::Theme(t) => s.set_theme(t),
        Item::Request(i) => {
            let rel = s.requests[i].rel.clone();
            s.select(&rel);
        }
    }
}

fn palette_key(s: &mut State, ev: &KeyEvent, _: &mut Tx) -> bool {
    let n = items(s).len();
    let Some(p) = &mut s.palette else {
        return false;
    };
    match ev.key {
        Key::Down if n > 0 => p.picked = (p.picked + 1) % n,
        Key::Up if n > 0 => p.picked = (p.picked + n - 1) % n,
        Key::Escape => s.palette = None,
        _ => return false,
    }
    true
}

fn palette(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(p) = &s.palette else { return };
    let (title, hint) = match p.mode {
        PaletteMode::Commands => ("Commands", "Type a command or env"),
        PaletteMode::Requests => ("Go to request", "Type a request name"),
    };
    let input = TextInput::new(&s.query)
        .width(None)
        .placeholder(hint)
        .leading_icon(k.glyph.search)
        .on_edit(|s: &mut State| &mut s.query)
        .on_change(|s: &mut State, _: &mut Tx| {
            if let Some(p) = &mut s.palette {
                p.picked = 0;
            }
        })
        .on_key(palette_key)
        .on_submit(|s: &mut State, tx: &mut Tx| {
            let at = s.palette.as_ref().map_or(0, |p| p.picked);
            if let Some(item) = items(s).into_iter().nth(at) {
                activate(s, item, tx);
            }
        })
        .build(k);
    let found = items(s);
    let mut list = col().key("results").gap(1.0);
    for (row_i, item) in found.iter().enumerate().take(12.max(p.picked + 1)) {
        let on = row_i == p.picked;
        let (label, detail, keys) = match item {
            Item::Command(i) => (
                COMMANDS[*i].title.to_owned(),
                String::new(),
                commands::label(&COMMANDS[*i]),
            ),
            Item::Env(e) => (
                format!("Env: {e}"),
                if *e == s.env {
                    "current".into()
                } else {
                    String::new()
                },
                None,
            ),
            Item::Theme(t) => (
                format!("Theme: {t}"),
                if s.cfg.ui.theme == *t {
                    "current".into()
                } else {
                    String::new()
                },
                None,
            ),
            Item::Request(i) => {
                let r = &s.requests[*i];
                let line = r
                    .primary()
                    .map(|e| format!("{} {}", e.method, e.url))
                    .unwrap_or_default();
                (r.rel.clone(), line, None)
            }
        };
        let picked = item.clone();
        let mut line = row()
            .key(format!("item-{row_i}"))
            .h(30.0)
            .px(10.0)
            .gap(10.0)
            .radius(6.0)
            .center_y()
            .fill(if on { k.c.bg_base } else { Color::TRANSPARENT })
            .child(text(label).font(k.ty.ui).color(k.c.text_base))
            .child(
                text(detail)
                    .font(k.ty.small)
                    .color(k.c.text_faint)
                    .grow(1.0)
                    .min_w(0.0),
            );
        if let Some(keys) = keys {
            line = line.child(text(keys).font(k.ty.small).color(k.c.text_muted));
        }
        list = list.child(
            line.on_press_tx(move |s: &mut State, tx: &mut Tx| activate(s, picked.clone(), tx)),
        );
    }
    if found.is_empty() {
        list = list.child(
            text("No matches")
                .font(k.ty.small)
                .color(k.c.text_faint)
                .px(10.0),
        );
    }
    Dialog::new("palette", title)
        .open(true)
        .width(600.0)
        .fit()
        .click_outside(true)
        .on_close(|s: &mut State| s.palette = None)
        .child(col().gap(8.0).child(input).child(list))
        .build(k, layer);
}

// --- Dialogs ---------------------------------------------------------------------------------

fn naming(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(what) = s.naming else { return };
    let (title, ok) = match what {
        Naming::New => ("New request", "Create"),
        Naming::Rename => ("Rename or move request", "Rename"),
        Naming::Duplicate => ("Duplicate request", "Duplicate"),
    };
    let input = TextInput::new(&s.name)
        .width(None)
        .placeholder("folder/name")
        .on_edit(|s: &mut State| &mut s.name)
        .on_submit(|s: &mut State, _: &mut Tx| s.finish_naming())
        .build(k);
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.naming = None)
                .build(k),
        )
        .child(
            Button::new(ok)
                .variant(ButtonVariant::Submit)
                .on_press(|s: &mut State| s.finish_naming())
                .build(k),
        );
    Dialog::new("naming", title)
        .description("A path under requests/; folders are created as needed.")
        .open(true)
        .width(460.0)
        .fit()
        .on_close(|s: &mut State| s.naming = None)
        .child(col().gap(12.0).padding(16.0).child(input).child(buttons))
        .build(k, layer);
}

fn confirm(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(what) = s.confirm else { return };
    let (title, desc, ok) = match what {
        Confirm::Delete => {
            let rel = s.current().map(|r| r.rel.clone()).unwrap_or_default();
            (
                format!("Delete {rel}?"),
                "The .hurl file and its run history are deleted.".to_owned(),
                "Delete",
            )
        }
        Confirm::ClearHistory => (
            "Delete all stored runs?".to_owned(),
            format!(
                "Every run in {} is deleted.",
                s.ws.state_dir().join("runs").display()
            ),
            "Delete runs",
        ),
    };
    let buttons = row()
        .w_full()
        .padding(16.0)
        .gap(8.0)
        .justify_end()
        .child(
            Button::new("Cancel")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.confirm = None)
                .build(k),
        )
        .child(
            Button::new(ok)
                .variant(ButtonVariant::Danger)
                .on_press(|s: &mut State| s.confirmed())
                .build(k),
        );
    Dialog::new("confirm", title)
        .description(desc)
        .open(true)
        .width(440.0)
        .fit()
        .on_close(|s: &mut State| s.confirm = None)
        .child(buttons)
        .build(k, layer);
}

fn settings(s: &State, k: &Kit, layer: &mut Vec<El>) {
    if !s.settings_open {
        return;
    }
    let switch = |key: &'static str, label: &str, desc: &str| {
        let toggle = Switch::new(s.setting(key) == "on")
            .label(label)
            .hide_label()
            .on_toggle(move |s: &mut State| {
                let on = s.setting(key) == "on";
                s.set(key, if on { "off" } else { "on" });
            })
            .build(k);
        row()
            .key(key)
            .gap(16.0)
            .center_y()
            .child(
                text(label.to_owned())
                    .font(k.ty.ui)
                    .color(k.c.text_base)
                    .w(150.0),
            )
            .child(toggle)
            .child(text(desc.to_owned()).font(k.ty.small).color(k.c.text_faint))
    };
    let sections = |key: &'static str, all: &'static [&'static str], label: &str| {
        let cur = s.setting(key);
        let have: Vec<&str> = cur.split(',').filter(|x| !x.is_empty()).collect();
        let mut r = row().gap(16.0).center_y().child(
            text(label.to_owned())
                .font(k.ty.ui)
                .color(k.c.text_base)
                .w(150.0),
        );
        for &sec in all {
            r = r.child(
                Checkbox::new(have.contains(&sec))
                    .label(sec)
                    .on_toggle(move |s: &mut State| {
                        let cur = s.setting(key);
                        let mut list: Vec<&str> =
                            cur.split(',').filter(|x| !x.is_empty()).collect();
                        if list.contains(&sec) {
                            list.retain(|x| *x != sec);
                        } else {
                            list = all
                                .iter()
                                .copied()
                                .filter(|a| *a == sec || list.contains(a))
                                .collect();
                        }
                        let v = list.join(",");
                        s.set(key, &v);
                    })
                    .build(k)
                    .key(format!("{key}-{sec}")),
            );
        }
        r
    };
    let envs: Vec<String> = std::iter::once("unset".to_owned())
        .chain(s.envs.iter().cloned())
        .collect();
    let default_env = row()
        .gap(16.0)
        .center_y()
        .child(
            text("Default env")
                .font(k.ty.ui)
                .color(k.c.text_base)
                .w(150.0),
        )
        .child(
            Select::new("default-env", &envs)
                .current(Some(&s.setting("run.default_env")))
                .appearance(SelectAppearance::Field)
                .width(200.0)
                .on_choose(|s: &mut State, v: &str, _: &mut Tx| s.set("run.default_env", v))
                .build(k, layer),
        );
    let at = THEMES.iter().position(|t| *t == s.cfg.ui.theme);
    let theme = row()
        .gap(16.0)
        .center_y()
        .child(text("Theme").font(k.ty.ui).color(k.c.text_base).w(150.0))
        .child(
            SegmentedControl::new(THEMES, at)
                .fit()
                .on_pick(|s: &mut State, i: usize, _: &mut Tx| s.set("ui.theme", THEMES[i]))
                .build(k)
                .key("theme"),
        );
    let unsaved = !s.unsaved.is_empty();
    let buttons = row()
        .w_full()
        .gap(8.0)
        .justify_end()
        .center_y()
        .child(
            text(if unsaved {
                "Changes apply now; save to keep them."
            } else {
                ""
            })
            .font(k.ty.small)
            .color(k.c.text_faint)
            .grow(1.0),
        )
        .child(
            Button::new("Save to config")
                .disabled(!unsaved)
                .on_press(|s: &mut State| s.save_settings())
                .build(k),
        )
        .child(
            Button::new("Done")
                .variant(ButtonVariant::Submit)
                .on_press(|s: &mut State| s.settings_open = false)
                .build(k),
        );
    let body = col()
        .gap(14.0)
        .padding(16.0)
        .child(theme)
        .child(switch(
            "ui.wrap",
            "Wrap lines",
            "Off scrolls long lines sideways.",
        ))
        .child(switch(
            "ui.auto_switch_response",
            "Show response",
            "Switch to the Response tab when a run finishes.",
        ))
        .child(switch(
            "run.asserts",
            "Asserts",
            "Run hurl's asserts instead of skipping them.",
        ))
        .child(default_env)
        .child(
            row()
                .gap(16.0)
                .center_y()
                .child(
                    text("Env files")
                        .font(k.ty.ui)
                        .color(k.c.text_base)
                        .w(150.0),
                )
                .child(
                    Button::new("Edit env files…")
                        .on_press(|s: &mut State| s.open_env_edit())
                        .build(k)
                        .key("edit-env"),
                ),
        )
        .child(sections(
            "request.sections",
            REQUEST_SECTIONS,
            "Request tab shows",
        ))
        .child(sections(
            "response.sections",
            RESPONSE_SECTIONS,
            "Response tab shows",
        ))
        .child(buttons);
    Dialog::new("settings", "Settings")
        .description(format!(
            "Shared with whurl: {}",
            whurl_core::config::Config::display_path()
        ))
        .open(true)
        .width(560.0)
        .fit()
        .on_close(|s: &mut State| s.settings_open = false)
        .child(body)
        .build(k, layer);
}

/// The env files dialog: the env's variables, its secrets and the shared secrets, as editors.
/// Secrets show as names over dots until revealed.
fn env_dialog(s: &State, k: &Kit, layer: &mut Vec<El>) {
    let Some(e) = &s.env_edit else { return };
    let paths = s.env_paths();
    let names: Vec<String> = paths
        .iter()
        .zip(&e.saved)
        .zip(&s.env_texts)
        .map(|((p, saved), t)| {
            let n = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if *saved != t.text {
                format!("{n} •")
            } else {
                n
            }
        })
        .collect();
    let labels: Vec<&str> = names.iter().map(String::as_str).collect();
    let tabs = SegmentedControl::new(&labels, Some(e.file))
        .fit()
        .on_pick(|s: &mut State, i: usize, _: &mut Tx| {
            if let Some(e) = &mut s.env_edit {
                e.file = i;
            }
        })
        .build(k)
        .key("env-files");
    let i = e.file;
    let secret = i > 0;
    let pane = if secret && !e.reveal {
        let masked: String = s.env_texts[i]
            .text
            .lines()
            .map(|l| match l.split_once('=') {
                Some((name, _)) if !l.trim_start().starts_with('#') => format!("{name}=••••••••"),
                _ => l.to_owned(),
            })
            .collect::<Vec<_>>()
            .join("\n");
        let shown = if masked.trim().is_empty() {
            "# no secrets yet".to_owned()
        } else {
            masked
        };
        col()
            .key("env-masked")
            .w_full()
            .min_h(220.0)
            .px(12.0)
            .py(10.0)
            .radius(k.radius.md)
            .fill(k.c.bg_base)
            .border(1.0, k.c.border_base)
            .child(text(shown).font(k.ty.mono).color(k.c.text_muted))
    } else {
        editor(
            s,
            k,
            &format!("env-{i}"),
            &s.env_texts[i],
            Some("dotenv"),
            move |s: &mut State| &mut s.env_texts[i],
            None,
        )
        .min_h(220.0)
    };
    let note = if !paths[i].exists() {
        format!(
            "env/{} doesn't exist yet: saving creates it.",
            labels[i].trim_end_matches(" •")
        )
    } else if i == 2 {
        "Shared by every env; an env's own secrets win.".to_owned()
    } else {
        "NAME=value per line; # starts a comment.".to_owned()
    };
    let mut buttons = row().w_full().gap(8.0).center_y().child(
        text(note)
            .font(k.ty.small)
            .color(k.c.text_faint)
            .grow(1.0)
            .min_w(0.0),
    );
    if secret {
        buttons = buttons.child(
            Button::new(if e.reveal {
                "Hide secrets"
            } else {
                "Show secrets"
            })
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| {
                if let Some(e) = &mut s.env_edit {
                    e.reveal = !e.reveal;
                }
            })
            .build(k)
            .key("env-reveal"),
        );
    }
    buttons = buttons
        .child(
            Button::new("Cancel")
                .on_press(|s: &mut State| s.close_env_edit())
                .build(k),
        )
        .child(
            Button::new("Save")
                .variant(ButtonVariant::Submit)
                .disabled(!s.env_dirty())
                .on_press(|s: &mut State| s.save_env_files())
                .build(k)
                .key("env-save"),
        );
    let body = col()
        .gap(12.0)
        .padding(16.0)
        .child(tabs)
        .child(col().w_full().max_h(380.0).child(pane).scroll(ScrollSpec {
            stick_bottom: false,
        }))
        .child(buttons);
    Dialog::new("env-edit", format!("Env files: {}", s.env))
        .description(
            "Read by whurl, the TUI and hurl runs. Secret values are masked in run output.",
        )
        .open(true)
        .width(640.0)
        .fit()
        .on_close(|s: &mut State| s.close_env_edit())
        .child(body)
        .build(k, layer);
}
