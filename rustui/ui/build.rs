//! Compiles the UI icons (`compositor/assets/ui/icons/lucide` and the pictures directly in `icons`) into the program, so the
//! status bar's glyphs and the weather pictures do not depend on a directory the helper's installer can remove.

use std::fmt::Write;
use std::path::Path;

fn pngs(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) {
    let Ok(read) = std::fs::read_dir(dir) else { return };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "png")
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
        {
            out.push((format!("{prefix}{name}"), path.to_string_lossy().replace('\\', "/")));
        }
    }
}

fn main() {
    let icons = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../compositor/assets/ui/icons");
    println!("cargo:rerun-if-changed={}", icons.display());
    println!("cargo:rerun-if-changed={}", icons.join("lucide").display());
    let mut files = Vec::new();
    pngs(&icons, "", &mut files);
    pngs(&icons.join("lucide"), "lucide/", &mut files);
    files.sort();
    let mut code = String::from("pub static ICONS: &[(&str, &[u8])] = &[\n");
    for (name, path) in &files {
        writeln!(code, "    ({name:?}, include_bytes!({path:?})),").unwrap();
    }
    code.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("embedded_icons.rs");
    std::fs::write(out, code).unwrap();
}
