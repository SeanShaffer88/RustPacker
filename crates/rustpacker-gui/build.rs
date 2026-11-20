//! Embed the manifest, version info and icon into Builder Studio.

#[path = "../../resources/win_resources.rs"]
mod win_resources;

fn main() {
    win_resources::emit(win_resources::AppInfo {
        product: "RustPacker Builder Studio",
        description: "RustPacker Builder Studio",
        exe_name: "rustpacker-gui.exe",
        assembly: "RustPacker.BuilderStudio",
        glyph: win_resources::IconGlyph::Build,
    });
}
