//! icons.json parsing and tray-state overlays.
//!
//! The 25 icon PNGs are used exactly as provided. Tray states only add
//! overlays around/on top of the unmodified image (ring, badge, dot) or
//! change opacity/saturation for the Processing and Off states.

use image::{imageops, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconEntry {
    /// Stable id (file stem), e.g. "swirl".
    pub id: String,
    /// File name inside the icons folder, e.g. "swirl.png".
    pub file: String,
    /// Display name.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconSet {
    pub icons: Vec<IconEntry>,
    pub default: String,
}

fn stem(file: &str) -> String {
    let f = file.rsplit(['/', '\\']).next().unwrap_or(file);
    f.rsplit_once('.').map(|(s, _)| s).unwrap_or(f).to_string()
}

fn title(id: &str) -> String {
    id.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn entry_from(v: &Value) -> Option<IconEntry> {
    match v {
        Value::String(s) => {
            let file = if s.contains('.') { s.clone() } else { format!("{s}.png") };
            let id = stem(&file);
            Some(IconEntry {
                name: title(&id),
                id,
                file,
            })
        }
        Value::Object(o) => {
            let get = |k: &str| o.get(k).and_then(Value::as_str).map(|s| s.to_string());
            let file = get("file")
                .or_else(|| get("filename"))
                .or_else(|| get("path"))
                .or_else(|| get("src"));
            let id = get("id").or_else(|| get("key")).or_else(|| get("slug"));
            let (id, file) = match (id, file) {
                (Some(i), Some(f)) => (i, f),
                (Some(i), None) => (i.clone(), format!("{i}.png")),
                (None, Some(f)) => (stem(&f), f),
                (None, None) => {
                    let n = get("name")?;
                    (
                        n.to_lowercase().replace(' ', "-"),
                        format!("{}.png", n.to_lowercase().replace(' ', "-")),
                    )
                }
            };
            let name = get("name")
                .or_else(|| get("label"))
                .or_else(|| get("title"))
                .unwrap_or_else(|| title(&id));
            Some(IconEntry { id, file, name })
        }
        _ => None,
    }
}

/// Parses icons.json. Accepts an array of names/objects, or an object with
/// an "icons"/"order" array and a "default" key (string id/file, or index).
pub fn parse_icons_json(json: &str) -> Result<IconSet, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("icons.json: {e}"))?;
    let (list, default_v) = match &v {
        Value::Array(a) => (a.clone(), None),
        Value::Object(o) => {
            let list = ["icons", "order", "items"]
                .iter()
                .find_map(|k| o.get(*k).and_then(Value::as_array).cloned())
                .ok_or("icons.json: no icon list found")?;
            (list, o.get("default").cloned())
        }
        _ => return Err("icons.json: unexpected format".into()),
    };
    let mut icons: Vec<IconEntry> = list.iter().filter_map(entry_from).collect();
    if icons.is_empty() {
        return Err("icons.json: icon list is empty".into());
    }
    // Per-item "default": true is also accepted.
    let item_default = list
        .iter()
        .position(|x| x.get("default").and_then(Value::as_bool) == Some(true));
    let default = match default_v {
        Some(Value::String(s)) => {
            let s_stem = stem(&s);
            icons
                .iter()
                .find(|i| i.id == s || i.file == s || i.id == s_stem)
                .map(|i| i.id.clone())
        }
        Some(Value::Number(n)) => n.as_u64().and_then(|i| icons.get(i as usize)).map(|i| i.id.clone()),
        _ => None,
    }
    .or_else(|| item_default.map(|i| icons[i].id.clone()))
    .or_else(|| icons.iter().find(|i| i.id == "swirl").map(|i| i.id.clone()))
    .unwrap_or_else(|| icons[0].id.clone());
    icons.dedup_by(|a, b| a.id == b.id);
    Ok(IconSet { icons, default })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrayState {
    Ready,
    Recording,
    Processing,
    Off,
    NeedsAttention,
    UpdateReady,
}

pub const RED: Rgba<u8> = Rgba([0xE5, 0x48, 0x4D, 0xFF]);
pub const AMBER: Rgba<u8> = Rgba([0xF2, 0xB6, 0x6B, 0xFF]);
pub const GREEN: Rgba<u8> = Rgba([0x7D, 0xDC, 0xAE, 0xFF]);
const DARK: Rgba<u8> = Rgba([0x11, 0x11, 0x16, 0xFF]);

/// Number of animation frames for the Processing state.
pub const PROCESSING_FRAMES: usize = 8;

fn blend(dst: &mut Rgba<u8>, src: Rgba<u8>, a: f32) {
    let a = a.clamp(0.0, 1.0);
    for i in 0..3 {
        dst[i] = (dst[i] as f32 * (1.0 - a) + src[i] as f32 * a).round() as u8;
    }
    dst[3] = (dst[3] as f32 + (255.0 - dst[3] as f32) * a).round() as u8;
}

fn draw_disc(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: Rgba<u8>) {
    let (w, h) = img.dimensions();
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cov = (r - d + 0.5).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend(img.get_pixel_mut(x, y), color, cov);
            }
        }
    }
}

fn draw_ring(img: &mut RgbaImage, cx: f32, cy: f32, r_out: f32, width: f32, color: Rgba<u8>) {
    let (w, h) = img.dimensions();
    let r_in = r_out - width;
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let cov = (r_out - d + 0.5).clamp(0.0, 1.0).min((d - r_in + 0.5).clamp(0.0, 1.0));
            if cov > 0.0 {
                blend(img.get_pixel_mut(x, y), color, cov);
            }
        }
    }
}

/// Builds a square tray image of `size` px for a state. The icon itself is
/// scaled but otherwise used as provided.
pub fn tray_image(icon: &RgbaImage, size: u32, state: TrayState, frame: usize) -> RgbaImage {
    let s = size as f32;
    let mut out = RgbaImage::new(size, size);
    let inner = match state {
        // Leave room for the ring so it doesn't cover the artwork.
        TrayState::Recording => (s * 0.72).round() as u32,
        _ => size,
    };
    let scaled = imageops::resize(icon, inner, inner, imageops::FilterType::Lanczos3);
    let off = ((size - inner) / 2) as i64;
    imageops::overlay(&mut out, &scaled, off, off);
    match state {
        TrayState::Ready => {}
        TrayState::Recording => {
            draw_ring(&mut out, s / 2.0, s / 2.0, s / 2.0, (s * 0.11).max(2.0), RED);
        }
        TrayState::Processing => {
            // Dimmed, gently pulsing between 40% and 75% opacity.
            let phase = (frame % PROCESSING_FRAMES) as f32 / PROCESSING_FRAMES as f32;
            let alpha = 0.575 + 0.175 * (phase * std::f32::consts::TAU).sin();
            for p in out.pixels_mut() {
                p[3] = (p[3] as f32 * alpha).round() as u8;
            }
        }
        TrayState::Off => {
            for p in out.pixels_mut() {
                let l = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8;
                *p = Rgba([l, l, l, (p[3] as f32 * 0.8) as u8]);
            }
        }
        TrayState::NeedsAttention => {
            let r = s * 0.2;
            draw_disc(&mut out, s - r, s - r, r + s * 0.04, DARK);
            draw_disc(&mut out, s - r, s - r, r, AMBER);
        }
        TrayState::UpdateReady => {
            let r = s * 0.17;
            draw_disc(&mut out, s - r - 0.5, r + 0.5, r + s * 0.04, DARK);
            draw_disc(&mut out, s - r - 0.5, r + 0.5, r, GREEN);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_object_with_default() {
        let s = parse_icons_json(r#"{"default":"swirl","icons":[{"id":"leaf","file":"leaf.png","name":"Leaf"},{"id":"swirl","file":"swirl.png"}]}"#).unwrap();
        assert_eq!(s.default, "swirl");
        assert_eq!(s.icons[0].id, "leaf");
        assert_eq!(s.icons[1].name, "Swirl");
    }

    #[test]
    fn parses_string_array_and_order_key() {
        let s = parse_icons_json(r#"["moon.png","swirl.png"]"#).unwrap();
        assert_eq!(s.default, "swirl");
        assert_eq!(
            s.icons.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["moon", "swirl"]
        );
        let s = parse_icons_json(r#"{"order":["a","swirl"],"default":"swirl.png"}"#).unwrap();
        assert_eq!(s.default, "swirl");
        assert_eq!(s.icons[0].file, "a.png");
        let s = parse_icons_json(r#"{"icons":[{"file":"x/sun.png"},{"file":"x/swirl.png","default":true}]}"#).unwrap();
        assert_eq!(s.default, "swirl");
        assert_eq!(s.icons[0].id, "sun");
    }

    fn sample() -> RgbaImage {
        RgbaImage::from_pixel(64, 64, Rgba([10, 120, 200, 255]))
    }

    #[test]
    fn tray_variants_differ_and_keep_size() {
        let icon = sample();
        let ready = tray_image(&icon, 32, TrayState::Ready, 0);
        assert_eq!(ready.dimensions(), (32, 32));
        assert_eq!(ready.get_pixel(16, 16), &Rgba([10, 120, 200, 255]));
        let rec = tray_image(&icon, 32, TrayState::Recording, 0);
        assert_eq!(rec.get_pixel(16, 1)[0], RED[0]);
        let off = tray_image(&icon, 32, TrayState::Off, 0);
        let p = off.get_pixel(16, 16);
        assert!(p[0] == p[1] && p[1] == p[2]);
        let a = tray_image(&icon, 32, TrayState::Processing, 0).get_pixel(16, 16)[3];
        let b = tray_image(&icon, 32, TrayState::Processing, 2).get_pixel(16, 16)[3];
        assert!(a < 255 && a != b);
        let att = tray_image(&icon, 32, TrayState::NeedsAttention, 0);
        assert_eq!(att.get_pixel(26, 26), &AMBER);
        let upd = tray_image(&icon, 32, TrayState::UpdateReady, 0);
        assert_eq!(upd.get_pixel(26, 5), &GREEN);
    }
}
