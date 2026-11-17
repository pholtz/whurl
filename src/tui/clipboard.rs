//! Copy to the system clipboard: a native tool when one exists (pbcopy, wl-copy, xclip, xsel),
//! else OSC 52, which asks the terminal to do it (works over ssh; tmux needs `set-clipboard on`).

use std::io::Write;
use std::process::{Command, Stdio};

/// Returns a note on how it was copied: empty for a native tool.
pub fn copy(text: &str) -> Result<&'static str, String> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let tools: &[(&str, &[&str])] = &[
        ("pbcopy", &[]),
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    for (bin, args) in tools {
        if *bin == "wl-copy" && !wayland {
            continue;
        }
        let Ok(mut child) = Command::new(bin).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        else {
            continue; // not installed
        };
        let wrote = child.stdin.take().map(|mut s| s.write_all(text.as_bytes()));
        match (wrote, child.wait()) {
            (Some(Ok(())), Ok(st)) if st.success() => return Ok(""),
            _ => return Err(format!("{bin} failed")),
        }
    }
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes())).and_then(|_| out.flush()).map_err(|e| e.to_string())?;
    Ok(" via the terminal (OSC 52)")
}

fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_pads() {
        assert_eq!(super::base64(b"hi"), "aGk=");
        assert_eq!(super::base64(b"hey"), "aGV5");
        assert_eq!(super::base64(b"h"), "aA==");
    }
}
