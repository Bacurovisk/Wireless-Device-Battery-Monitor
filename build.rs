// Embeds the icon, res/app.manifest (Windows 10 compatibility - required for the taskbar strip's
// layered child window - per-monitor DPI awareness, common controls 6) and version info.
#[path = "res/resources.rs"]
mod resources;

fn main() {
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("res");
    resources::embed(&dir, "BatteryHub - battery levels of wireless devices");
}
