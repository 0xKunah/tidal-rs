//! Build script: embeds the Windows icon.

fn main() {
	if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
		println!("cargo:rerun-if-changed=assets/icon.ico");
		let mut resources = winresource::WindowsResource::new();
		// Resource id 1: Explorer, the taskbar and GPUI's window icon all use it.
		resources.set_icon("assets/icon.ico");
		resources
			.compile()
			.expect("failed to compile the Windows resources");
	}
}
