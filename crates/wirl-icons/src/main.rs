//! Icon tooling.
//!
//!   cargo run -p wirl-icons -- import     unzip wirl-icons.zip into ui/assets/icons and delete the zip
//!   cargo run -p wirl-icons -- generate   build src-tauri/icons/* (app .ico/.png) from the default icon
//!   cargo run -p wirl-icons -- check      verify ui/assets/icons is complete
//!
//! The PNGs are used exactly as provided: generation only scales them to the
//! sizes Windows needs. Nothing is redrawn or restyled.

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{imageops, ExtendedColorType, RgbaImage};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use wirl_core::icons::{parse_icons_json, IconSet};

const ICONS_DIR: &str = "ui/assets/icons";
const ZIP: &str = "wirl-icons.zip";
const OUT_DIR: &str = "src-tauri/icons";

fn root() -> PathBuf {
    // Run from anywhere inside the repo.
    let mut p = std::env::current_dir().expect("cwd");
    loop {
        if p.join("Cargo.toml").exists() && p.join("ui").exists() {
            return p;
        }
        if !p.pop() {
            return std::env::current_dir().expect("cwd");
        }
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("wirl-icons: {msg}");
    std::process::exit(1)
}

fn import(root: &Path) {
    let zip_path = root.join(ZIP);
    let dest = root.join(ICONS_DIR);
    if !zip_path.exists() {
        if dest.join("icons.json").exists() {
            println!("{ZIP} already imported into {ICONS_DIR}");
            return;
        }
        fail(&format!("{ZIP} not found in the repo root"));
    }
    let f = fs::File::open(&zip_path).unwrap_or_else(|e| fail(&format!("open zip: {e}")));
    let mut z = zip::ZipArchive::new(f).unwrap_or_else(|e| fail(&format!("read zip: {e}")));
    fs::create_dir_all(&dest).unwrap();
    let mut n = 0;
    for i in 0..z.len() {
        let mut file = z.by_index(i).unwrap();
        if file.is_dir() {
            continue;
        }
        // Flatten and keep only the base name (also prevents path traversal).
        let name = Path::new(file.name())
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let lower = name.to_lowercase();
        if name.starts_with('.') || !(lower.ends_with(".png") || lower == "icons.json") {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf).unwrap();
        fs::write(dest.join(&name), buf).unwrap();
        n += 1;
    }
    check(root);
    fs::remove_file(&zip_path).unwrap_or_else(|e| fail(&format!("delete zip: {e}")));
    println!("Imported {n} files into {ICONS_DIR} and deleted {ZIP}");
}

fn load_set(root: &Path) -> IconSet {
    let json = fs::read_to_string(root.join(ICONS_DIR).join("icons.json")).unwrap_or_else(|_| {
        fail(&format!(
            "{ICONS_DIR}/icons.json missing. Add {ZIP} to the repo root and run `cargo run -p wirl-icons -- import`."
        ))
    });
    parse_icons_json(&json).unwrap_or_else(|e| fail(&e))
}

fn check(root: &Path) -> IconSet {
    let set = load_set(root);
    for i in &set.icons {
        let p = root.join(ICONS_DIR).join(&i.file);
        if !p.exists() {
            fail(&format!("icons.json lists {} but {} is missing", i.file, p.display()));
        }
        image::open(&p).unwrap_or_else(|e| fail(&format!("{}: {e}", i.file)));
    }
    println!("{} icons, default \"{}\"", set.icons.len(), set.default);
    set
}

fn scaled(img: &RgbaImage, size: u32) -> RgbaImage {
    if img.width() == size && img.height() == size {
        img.clone()
    } else {
        imageops::resize(img, size, size, imageops::FilterType::Lanczos3)
    }
}

fn generate(root: &Path) {
    let set = check(root);
    let def = set.icons.iter().find(|i| i.id == set.default).expect("default listed");
    let img = image::open(root.join(ICONS_DIR).join(&def.file)).unwrap().to_rgba8();
    let out = root.join(OUT_DIR);
    fs::create_dir_all(&out).unwrap();
    for (name, size) in [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
    ] {
        scaled(&img, size).save(out.join(name)).unwrap();
    }
    let frames: Vec<IcoFrame> = [16u32, 20, 24, 32, 40, 48, 64, 128, 256]
        .iter()
        .map(|s| {
            let im = scaled(&img, *s);
            IcoFrame::as_png(im.as_raw(), *s, *s, ExtendedColorType::Rgba8).unwrap()
        })
        .collect();
    let f = fs::File::create(out.join("icon.ico")).unwrap();
    IcoEncoder::new(f).encode_images(&frames).unwrap();
    println!("Wrote app icons to {OUT_DIR} from {}", def.file);
}

fn main() {
    let root = root();
    match std::env::args().nth(1).as_deref() {
        Some("import") => import(&root),
        Some("generate") => generate(&root),
        Some("check") => {
            check(&root);
        }
        Some("all") | None => {
            import(&root);
            generate(&root);
        }
        Some(other) => fail(&format!("unknown command {other}; use import, generate, check or all")),
    }
}
