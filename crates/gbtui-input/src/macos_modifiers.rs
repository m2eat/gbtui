//! Native macOS modifier detection via CoreGraphics.
//!
//! `SOURCE: crates/codegen/xai-grok-pager-render/src/input/macos_modifiers.rs (COPY)`

use crate::normalizer::ModifierState;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSourceFlagsState(state_id: i32) -> u64;
}

const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;
const K_CG_EVENT_FLAG_MASK_SHIFT: u64 = 0x0002_0000;
const K_CG_EVENT_FLAG_MASK_CONTROL: u64 = 0x0004_0000;
const K_CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x0008_0000;
const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 0x0010_0000;

fn flags() -> u64 {
    // SAFETY: CGEventSourceFlagsState is a stable public API (macOS 10.4+).
    unsafe { CGEventSourceFlagsState(K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE) }
}

pub fn snapshot() -> ModifierState {
    let f = flags();
    ModifierState {
        command: f & K_CG_EVENT_FLAG_MASK_COMMAND != 0,
        option: f & K_CG_EVENT_FLAG_MASK_ALTERNATE != 0,
        shift: f & K_CG_EVENT_FLAG_MASK_SHIFT != 0,
        control: f & K_CG_EVENT_FLAG_MASK_CONTROL != 0,
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn smoke_test_modifier_detection() {
        let s = snapshot();
        let _ = (s.command, s.option, s.shift, s.control);
    }
}
