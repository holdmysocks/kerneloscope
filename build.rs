// Embeds every file in assets/ into the .efi so the menu is a single self-contained file.
use std::{env, fs, path::Path};

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<_> = fs::read_dir(&dir)
        .expect("assets/ missing: run tools/export-assets.ps1")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .collect();
    names.sort();
    let mut src = String::from("pub static ASSETS: &[(&str, &[u8])] = &[\n");
    for p in names {
        let stem = p.file_stem().unwrap().to_string_lossy();
        src += &format!("    ({:?}, include_bytes!({:?})),\n", stem, p.display().to_string());
    }
    src += "];\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("assets.rs"), src).unwrap();
}
