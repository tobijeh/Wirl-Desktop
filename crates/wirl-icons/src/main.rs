//! Icon tooling.
//!
//!   cargo run -p wirl-icons -- import     unzip wirl-icons.zip into ui/assets/icons and delete the zip
//!                                         (or, without a zip, cut wirl-icons-sheet.png into the icons)
//!   cargo run -p wirl-icons -- slice      cut wirl-icons-sheet.png (5x5 grid) into ui/assets/icons
//!   cargo run -p wirl-icons -- generate   build src-tauri/icons/* (app .ico/.png) from the default icon
//!   cargo run -p wirl-icons -- check      verify ui/assets/icons is complete
//!
//! The PNGs are used exactly as provided: slicing copies each icon's pixels
//! out of the sheet unchanged, and generation only scales them to the sizes
//! Windows needs. Nothing is redrawn or restyled.

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{imageops, ExtendedColorType, RgbaImage};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use wirl_core::icons::{parse_icons_json, IconSet};

const ICONS_DIR: &str = "ui/assets/icons";
const ZIP: &str = "wirl-icons.zip";
const OUT_DIR: &str = "src-tauri/icons";
const SHEET: &str = "wirl-icons-sheet.png";

/// The icons on wirl-icons-sheet.png, left to right, top to bottom.
/// The first one (the Wirl swirl) is the default.
const SHEET_ICONS: [(&str, &str); 25] = [
    ("swirl", "Swirl"),
    ("microphone", "Microphone"),
    ("book", "Book"),
    ("text-sparkle", "Text Sparkle"),
    ("send", "Send"),
    ("keyboard", "Keyboard"),
    ("message", "Message"),
    ("sparkles", "Sparkles"),
    ("sprout", "Sprout"),
    ("paw", "Paw"),
    ("palette", "Palette"),
    ("wave", "Wave"),
    ("mountains", "Mountains"),
    ("tree", "Tree"),
    ("butterfly", "Butterfly"),
    ("bee", "Bee"),
    ("hummingbird", "Hummingbird"),
    ("dolphin", "Dolphin"),
    ("seahorse", "Seahorse"),
    ("jellyfish", "Jellyfish"),
    ("whale", "Whale"),
    ("turtle", "Turtle"),
    ("hummingbird-2", "Hummingbird 2"),
    ("rocket", "Rocket"),
    ("pizza", "Pizza"),
];
const SHEET_GRID: u32 = 5;

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

/// Bounding box of the non-transparent pixels in a region.
fn alpha_bbox(img: &RgbaImage, x0: u32, y0: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let (mut l, mut t, mut r, mut b) = (u32::MAX, u32::MAX, 0, 0);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            if img.get_pixel(x, y)[3] > 0 {
                l = l.min(x);
                t = t.min(y);
                r = r.max(x);
                b = b.max(y);
            }
        }
    }
    (l != u32::MAX).then(|| (l, t, r - l + 1, b - t + 1))
}

/// Cuts the 5x5 icon sheet into one PNG per icon plus icons.json. Each icon
/// is copied pixel for pixel (no resampling), cropped to a square around its
/// visible pixels.
fn slice(root: &Path) {
    let sheet_path = root.join(SHEET);
    let sheet = image::open(&sheet_path)
        .unwrap_or_else(|e| fail(&format!("{SHEET}: {e}")))
        .to_rgba8();
    let (w, h) = sheet.dimensions();
    if w % SHEET_GRID != 0 || h % SHEET_GRID != 0 {
        fail(&format!(
            "{SHEET} is {w}x{h}; expected a {SHEET_GRID}x{SHEET_GRID} grid of equal cells"
        ));
    }
    let (cw, ch) = (w / SHEET_GRID, h / SHEET_GRID);
    let dest = root.join(ICONS_DIR);
    fs::create_dir_all(&dest).unwrap();
    let mut list = Vec::new();
    for (i, (id, name)) in SHEET_ICONS.iter().enumerate() {
        let (cx, cy) = ((i as u32 % SHEET_GRID) * cw, (i as u32 / SHEET_GRID) * ch);
        let (x, y, bw, bh) =
            alpha_bbox(&sheet, cx, cy, cw, ch).unwrap_or_else(|| fail(&format!("{SHEET}: cell {i} ({id}) is empty")));
        // Square crop centred on the icon, kept inside the cell.
        let side = bw.max(bh);
        let sx = (x + bw / 2).saturating_sub(side / 2).clamp(cx, cx + cw - side);
        let sy = (y + bh / 2).saturating_sub(side / 2).clamp(cy, cy + ch - side);
        let icon = imageops::crop_imm(&sheet, sx, sy, side, side).to_image();
        let file = format!("{id}.png");
        icon.save(dest.join(&file)).unwrap();
        list.push(serde_json::json!({ "id": id, "file": file, "name": name }));
    }
    let json = serde_json::json!({ "default": SHEET_ICONS[0].0, "icons": list });
    fs::write(
        dest.join("icons.json"),
        serde_json::to_string_pretty(&json).unwrap() + "\n",
    )
    .unwrap();
    println!("Cut {} icons from {SHEET} into {ICONS_DIR}", SHEET_ICONS.len());
    check(root);
}

fn import(root: &Path) {
    let zip_path = root.join(ZIP);
    let dest = root.join(ICONS_DIR);
    if !zip_path.exists() {
        if dest.join("icons.json").exists() {
            println!("Icons already present in {ICONS_DIR}");
            return;
        }
        if root.join(SHEET).exists() {
            slice(root);
            return;
        }
        fail(&format!("neither {ZIP} nor {SHEET} found in the repo root"));
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
            "{ICONS_DIR}/icons.json missing. Add {ZIP} or {SHEET} to the repo root and run `cargo run -p wirl-icons -- import`."
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
        Some("slice") => slice(&root),
        Some("generate") => generate(&root),
        Some("check") => {
            check(&root);
        }
        Some("all") | None => {
            import(&root);
            generate(&root);
        }
        Some(other) => fail(&format!(
            "unknown command {other}; use import, slice, generate, check or all"
        )),
    }
}
