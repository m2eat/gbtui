//! System clipboard provider for [`TextArea`](crate::TextArea).
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/clipboard (STRIP)` —
//! arboard + optional OSC 52. No `GROK_COPY_FILE`, `grok wrap`, or remote
//! session probing.
//!
//! Hosts inject this with [`crate::TextArea::set_clipboard_provider`] /
//! [`crate::Prompt::set_clipboard_provider`].
//!
//! Feature `clipboard` (default) enables arboard. Without it this type still
//! compiles and stores text in [`InternalClipboard`].

use std::io::Write;

use crate::textarea::{ClipboardProvider, InternalClipboard};

/// OSC 52 clipboard-set: `ESC ] 52 ; c ; <base64> BEL`.
///
/// When `tmux_passthrough` is true the sequence is wrapped in a tmux DCS
/// passthrough (`ESC P tmux ; ESC … BEL ESC \`). Only correct when tmux is
/// the *immediate* terminal.
pub fn osc52_set_sequence(text: &str, tmux_passthrough: bool) -> Vec<u8> {
    let encoded = b64_encode(text.as_bytes());
    if tmux_passthrough {
        format!("\x1bPtmux;\x1b\x1b]52;c;{encoded}\x07\x1b\\").into_bytes()
    } else {
        format!("\x1b]52;c;{encoded}\x07").into_bytes()
    }
}

/// Native clipboard when the `clipboard` feature is on, otherwise an
/// in-memory buffer. OSC 52 is an optional Unix write fallback.
#[derive(Debug)]
pub struct SystemClipboard {
    fallback: InternalClipboard,
    /// Talk to arboard (no-op when the `clipboard` feature is off).
    native: bool,
    /// After a failed native write, emit OSC 52 on Unix.
    osc52_fallback: bool,
}

impl Default for SystemClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemClipboard {
    /// Native clipboard (if compiled in) plus OSC 52 fallback on Unix.
    pub fn new() -> Self {
        Self {
            fallback: InternalClipboard::default(),
            native: true,
            osc52_fallback: cfg!(unix),
        }
    }

    /// In-memory only. Useful in tests and headless hosts.
    pub fn memory_only() -> Self {
        Self {
            fallback: InternalClipboard::default(),
            native: false,
            osc52_fallback: false,
        }
    }

    pub fn with_osc52(mut self, enabled: bool) -> Self {
        self.osc52_fallback = enabled;
        self
    }

    pub fn with_native(mut self, enabled: bool) -> Self {
        self.native = enabled;
        self
    }

    fn try_native_set(&self, text: &str) -> bool {
        if !self.native {
            return false;
        }
        #[cfg(feature = "clipboard")]
        {
            if let Ok(mut clip) = arboard::Clipboard::new() {
                return clip.set_text(text).is_ok();
            }
        }
        #[cfg(not(feature = "clipboard"))]
        {
            let _ = text;
        }
        false
    }
}

impl ClipboardProvider for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        if self.native {
            #[cfg(feature = "clipboard")]
            {
                if let Ok(mut clip) = arboard::Clipboard::new()
                    && let Ok(text) = clip.get_text()
                {
                    return Some(text);
                }
            }
        }
        self.fallback.get()
    }

    fn set(&mut self, text: &str) {
        if !self.try_native_set(text) && self.osc52_fallback {
            emit_osc52(text);
        }
        self.fallback.set(text);
    }
}

fn emit_osc52(text: &str) {
    let tmux = std::env::var_os("TMUX").is_some();
    let seq = osc52_set_sequence(text, tmux);
    let stderr = std::io::stderr();
    let mut handle = stderr.lock();
    let _ = handle.write_all(&seq);
    let _ = handle.flush();
}

fn b64_encode(data: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };
        out.push(T[(b0 >> 2) as usize] as char);
        out.push(T[(((b0 & 3) << 4) | (b1 >> 4)) as usize] as char);
        if i + 1 < data.len() {
            out.push(T[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < data.len() {
            out.push(T[(b2 & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_plain_encodes_hi() {
        assert_eq!(osc52_set_sequence("hi", false), b"\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn osc52_tmux_passthrough_wraps() {
        assert_eq!(
            osc52_set_sequence("hi", true),
            b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x07\x1b\\"
        );
    }

    #[test]
    fn memory_only_roundtrips() {
        let mut c = SystemClipboard::memory_only();
        c.set("hello");
        assert_eq!(c.get().as_deref(), Some("hello"));
    }

    #[test]
    fn empty_payload_is_valid_b64() {
        assert_eq!(osc52_set_sequence("", false), b"\x1b]52;c;\x07");
    }
}
