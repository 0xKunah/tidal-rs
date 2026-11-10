//! Colors from the approved mockup that the component theme does not cover.

use gpui_kit::*;

pub fn red() -> Hsla {
	rgb(0xdc2626).into()
}

pub fn red_tint() -> Hsla {
	rgba(0xdc262624).into()
}

pub fn panel() -> Hsla {
	rgb(0x0b0b0d).into()
}

pub fn muted() -> Hsla {
	rgb(0x18181b).into()
}

pub fn accent() -> Hsla {
	rgb(0x27272a).into()
}

pub fn border() -> Hsla {
	rgb(0x232326).into()
}

pub fn fg() -> Hsla {
	rgb(0xfafafa).into()
}

pub fn fg_muted() -> Hsla {
	rgb(0xa1a1aa).into()
}

pub fn fg_dim() -> Hsla {
	rgb(0x71717a).into()
}
