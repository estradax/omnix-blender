//! Embeds every .py/.toml file of the add-on so `install-addon` works from a plain `cargo install`.
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    for e in std::fs::read_dir(dir).expect("read addon dir") {
        let p = e.expect("entry").path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != "__pycache__") {
                walk(&p, base, out);
            }
        } else if p.extension().is_some_and(|x| x == "py" || x == "toml") {
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    let root =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("addon/omnix_blender");
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    files.sort();
    let mut src = String::from("pub const ADDON_FILES: &[(&str, &str)] = &[\n");
    for (rel, abs) in &files {
        src += &format!(
            "    ({rel:?}, include_str!({:?})),\n",
            abs.to_string_lossy()
        );
    }
    src += "];\n";
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("addon_files.rs");
    std::fs::write(out, src).unwrap();
    println!("cargo:rerun-if-changed=addon/omnix_blender");
}
