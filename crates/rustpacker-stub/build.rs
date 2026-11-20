//! Embed the manifest, version info and icon into the installer stub.
//!
//! The machinery lives in `resources/win_resources.rs` so the Builder Studio
//! gets the same treatment without duplicating it; this file only says which
//! binary it is.

#[path = "../../resources/win_resources.rs"]
mod win_resources;

fn main() {
    win_resources::emit(win_resources::AppInfo {
        product: "RustPacker",
        description: "RustPacker Setup",
        exe_name: "rustpacker-stub.exe",
        assembly: "RustPacker.Setup",
        glyph: win_resources::IconGlyph::Install,
    });
}
