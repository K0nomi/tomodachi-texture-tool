use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{imageops, DynamicImage, ImageEncoder};
use serde::Serialize;

// ─── Item type file-name templates ───────────────────────────────────────────

struct ItemType {
    key:    &'static str,
    canvas: &'static str,
    ugctex: &'static str,
    thumb:  &'static str,
}

const TYPES: &[ItemType] = &[
    ItemType { key: "facepaint", canvas: "UgcFacePaint{id}.canvas.zs",  ugctex: "UgcFacePaint{id}.ugctex.zs",  thumb: "UgcFacePaint{id}_Thumb.ugctex.zs" },
    ItemType { key: "goods",     canvas: "UgcGoods{id}.canvas.zs",      ugctex: "UgcGoods{id}.ugctex.zs",      thumb: "UgcGoods{id}_Thumb.ugctex.zs"     },
    ItemType { key: "clothes",   canvas: "UgcCloth{id}.canvas.zs",      ugctex: "UgcCloth{id}.ugctex.zs",      thumb: "UgcCloth{id}_Thumb.ugctex.zs"     },
    ItemType { key: "exterior",  canvas: "UgcExterior{id}.canvas.zs",   ugctex: "UgcExterior{id}.ugctex.zs",   thumb: "UgcExterior{id}_Thumb.ugctex.zs"  },
    ItemType { key: "interior",  canvas: "UgcInterior{id}.canvas.zs",   ugctex: "UgcInterior{id}.ugctex.zs",   thumb: "UgcInterior{id}_Thumb.ugctex.zs"  },
    ItemType { key: "mapobject", canvas: "UgcMapObject{id}.canvas.zs",  ugctex: "UgcMapObject{id}.ugctex.zs",  thumb: "UgcMapObject{id}_Thumb.ugctex.zs" },
    ItemType { key: "mapfloor",  canvas: "UgcMapFloor{id}.canvas.zs",   ugctex: "UgcMapFloor{id}.ugctex.zs",   thumb: "UgcMapFloor{id}_Thumb.ugctex.zs"  },
    ItemType { key: "food",      canvas: "UgcFood{id}.canvas.zs",       ugctex: "UgcFood{id}.ugctex.zs",       thumb: "UgcFood{id}_Thumb.ugctex.zs"      },
];

fn tmpl(item_type: &str) -> Option<&'static ItemType> {
    TYPES.iter().find(|t| t.key == item_type)
}

fn apply_id(template: &str, id_str: &str) -> String {
    template.replace("{id}", id_str)
}

// ─── Nintendo Switch GOB swizzle (port of Aclios/pyswizzle) ─────────────────

fn compute_tile_perm(tile_rows: usize, ops: &[(usize, usize)]) -> Vec<(usize, usize)> {
    const TILE_COLS: usize = 4;
    let init: Vec<Vec<(usize, usize)>> = (0..tile_rows)
        .map(|r| (0..TILE_COLS).map(|c| (r, c)).collect())
        .collect();
    let mut parts: Vec<Vec<Vec<(usize, usize)>>> = vec![init];
    for &(n, axis) in ops {
        let mut next = Vec::with_capacity(parts.len() * n);
        for grid in &parts {
            let rows = grid.len();
            let cols = if rows > 0 { grid[0].len() } else { 0 };
            if axis == 0 {
                let sub = rows / n;
                for i in 0..n { next.push(grid[i * sub..(i + 1) * sub].to_vec()); }
            } else {
                let sub = cols / n;
                for i in 0..n {
                    next.push(grid.iter().map(|row| row[i * sub..(i + 1) * sub].to_vec()).collect());
                }
            }
        }
        parts = next;
    }
    parts.iter().map(|g| g[0][0]).collect()
}

fn nsw_swizzle(data: &[u8], im_width: usize, im_height: usize,
               block_width: usize, block_height: usize,
               bytes_per_block: usize, swizzle_mode: usize) -> Vec<u8> {
    const READ: usize = 16;
    const TILE_COLS: usize = 4;
    let column_count    = (bytes_per_block * im_width) / (block_width * READ);
    let tile_width      = (64 / bytes_per_block) * block_width;
    let tile_height     = 8 * block_height * (1 << swizzle_mode);
    let tile_per_width  = im_width  / tile_width;
    let tile_per_height = im_height / tile_height;
    let row_count       = im_height / block_height;
    let tile_rows       = row_count / tile_per_height;
    let ops: &[(usize, usize)] = &[(1 << swizzle_mode, 0), (2, 1), (4, 0), (2, 1), (2, 0)];
    let perm = compute_tile_perm(tile_rows, ops);
    let mut grid: Vec<Vec<[u8; READ]>> = vec![vec![[0u8; READ]; column_count]; row_count];
    for r in 0..row_count {
        for c in 0..column_count {
            let base = (r * column_count + c) * READ;
            grid[r][c].copy_from_slice(&data[base..base + READ]);
        }
    }
    let mut out = vec![0u8; data.len()];
    let mut pos = 0usize;
    for ty in 0..tile_per_height {
        for tx in 0..tile_per_width {
            for &(pr, pc) in &perm {
                let r = ty * tile_rows + pr;
                let c = tx * TILE_COLS + pc;
                out[pos..pos + READ].copy_from_slice(&grid[r][c]);
                pos += READ;
            }
        }
    }
    out
}

fn nsw_unswizzle(data: &[u8], im_width: usize, im_height: usize,
                 block_width: usize, block_height: usize,
                 bytes_per_block: usize, swizzle_mode: usize) -> Vec<u8> {
    const READ: usize = 16;
    const TILE_COLS: usize = 4;
    let column_count    = (bytes_per_block * im_width) / (block_width * READ);
    let tile_width      = (64 / bytes_per_block) * block_width;
    let tile_height     = 8 * block_height * (1 << swizzle_mode);
    let tile_per_width  = im_width  / tile_width;
    let tile_per_height = im_height / tile_height;
    let row_count       = im_height / block_height;
    let tile_rows       = row_count / tile_per_height;
    let ops: &[(usize, usize)] = &[(1 << swizzle_mode, 0), (2, 1), (4, 0), (2, 1), (2, 0)];
    let perm = compute_tile_perm(tile_rows, ops);
    let mut grid: Vec<Vec<[u8; READ]>> = vec![vec![[0u8; READ]; column_count]; row_count];
    let mut pos = 0usize;
    for ty in 0..tile_per_height {
        for tx in 0..tile_per_width {
            for &(pr, pc) in &perm {
                let r = ty * tile_rows + pr;
                let c = tx * TILE_COLS + pc;
                grid[r][c].copy_from_slice(&data[pos..pos + READ]);
                pos += READ;
            }
        }
    }
    let mut out = vec![0u8; data.len()];
    for r in 0..row_count {
        for c in 0..column_count {
            let base = (r * column_count + c) * READ;
            out[base..base + READ].copy_from_slice(&grid[r][c]);
        }
    }
    out
}

// ─── Colour-space & resize helpers ───────────────────────────────────────────

/// sRGB u8 → linear u8  (applies IEC 61966-2-1 transfer function).
/// Used on R, G, B — alpha is always kept as-is.
#[inline]
fn srgb_to_lin(v: u8) -> u8 {
    let f = v as f32 / 255.0;
    let l = if f <= 0.04045 { f / 12.92 } else { ((f + 0.055) / 1.055).powf(2.4) };
    (l * 255.0 + 0.5) as u8
}

/// Resize `img` to `w × h` with two corrections applied:
///
/// 1. **sRGB → linear** (fixes issue #2 — brightness/gamma shift):
///    The game reads raw texture bytes as linear light.  Storing sRGB values
///    makes colours look ~30-50 % brighter than intended.
///
/// 2. **Premultiplied-alpha Lanczos** (fixes issue #3 — fringing on edges):
///    Resampling straight-alpha with Lanczos creates dark halos around
///    semi-transparent pixels because RGB and alpha are filtered independently.
///    Premultiplying first couples them correctly.
///
/// Returns linear straight-alpha RGBA, ready for BC compression / swizzle.
fn prepare_rgba(img: &DynamicImage, w: u32, h: u32) -> Vec<u8> {
    let src = img.to_rgba8();
    let (sw, sh) = (img.width(), img.height());

    // straight sRGB  →  premultiplied linear
    let pre_lin: Vec<u8> = src.chunks_exact(4).flat_map(|p| {
        let a = p[3] as f32 / 255.0;
        [
            (srgb_to_lin(p[0]) as f32 * a + 0.5) as u8,
            (srgb_to_lin(p[1]) as f32 * a + 0.5) as u8,
            (srgb_to_lin(p[2]) as f32 * a + 0.5) as u8,
            p[3],
        ]
    }).collect();

    let src_img = image::RgbaImage::from_raw(sw, sh, pre_lin).expect("dims valid");
    let resized = imageops::resize(&src_img, w, h, imageops::FilterType::Lanczos3);

    // premultiplied linear  →  straight linear  (game expects linear)
    resized.chunks_exact(4).flat_map(|p| {
        let a = p[3] as f32 / 255.0;
        if a < f32::EPSILON {
            [0u8, 0, 0, p[3]]
        } else {
            [
                (p[0] as f32 / a + 0.5).min(255.0) as u8,
                (p[1] as f32 / a + 0.5).min(255.0) as u8,
                (p[2] as f32 / a + 0.5).min(255.0) as u8,
                p[3],
            ]
        }
    }).collect()
}

// ─── Block compression / decompression ───────────────────────────────────────

fn bc_compress(fmt: squish::Format, rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut out = vec![0u8; fmt.compressed_size(w, h)];
    fmt.compress(rgba, w, h, squish::Params { algorithm: squish::Algorithm::IterativeClusterFit, ..Default::default() }, &mut out);
    out
}

fn bc1_compress(rgba: &[u8], w: u32, h: u32) -> Vec<u8> { bc_compress(squish::Format::Bc1, rgba, w, h) }
fn bc3_compress(rgba: &[u8], w: u32, h: u32) -> Vec<u8> { bc_compress(squish::Format::Bc3, rgba, w, h) }

fn bc3_decompress(data: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = vec![0u8; width as usize * height as usize * 4];
    squish::Format::Bc3.decompress(data, width as usize, height as usize, &mut out);
    out
}

// ─── Per-format encoding ──────────────────────────────────────────────────────

fn to_canvas(img: &DynamicImage) -> Vec<u8> {
    // canvas = raw linear RGBA 256×256, swizzled
    nsw_swizzle(&prepare_rgba(img, 256, 256), 256, 256, 1, 1, 4, 4)
}

fn to_ugctex(img: &DynamicImage) -> Vec<u8> {
    // ugctex = BC1-compressed linear 512×512, swizzled
    let rgba = prepare_rgba(img, 512, 512);
    nsw_swizzle(&bc1_compress(&rgba, 512, 512), 512, 512, 4, 4, 8, 4)
}

fn to_thumb(img: &DynamicImage) -> Vec<u8> {
    // thumb = BC3-compressed linear 256×256, swizzled
    let rgba = prepare_rgba(img, 256, 256);
    nsw_swizzle(&bc3_compress(&rgba, 256, 256), 256, 256, 4, 4, 16, 3)
}

fn zstd_compress(data: &[u8]) -> std::io::Result<Vec<u8>> {
    zstd::encode_all(std::io::Cursor::new(data), 16)
}

// ─── Thumbnail decoding for previews ─────────────────────────────────────────

fn thumb_to_preview(zs_data: &[u8]) -> Option<String> {
    let raw = zstd::decode_all(std::io::Cursor::new(zs_data)).ok()?;
    if raw.len() != squish::Format::Bc3.compressed_size(256, 256) { return None; }
    let linear = nsw_unswizzle(&raw, 256, 256, 4, 4, 16, 3);
    let rgba   = bc3_decompress(&linear, 256, 256);
    let img    = image::RgbaImage::from_raw(256, 256, rgba)?;
    let small  = imageops::resize(&img, 64, 64, imageops::FilterType::Triangle);
    let mut buf = Vec::new();
    image::codecs::png::PngEncoder::new(&mut buf)
        .write_image(small.as_raw(), 64, 64, image::ExtendedColorType::Rgba8).ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(&buf)))
}

// ─── Emulator auto-detection ─────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct FoundSave {
    pub emulator: String,
    pub path:     String,
}

fn emulator_save_roots() -> Vec<(&'static str, PathBuf)> {
    let mut v = Vec::new();

    #[cfg(target_os = "windows")]
    if let Ok(appdata) = std::env::var("APPDATA") {
        let base = PathBuf::from(appdata);
        v.push(("Ryujinx", base.join("Ryujinx").join("bis").join("user").join("save")));
        v.push(("Eden",    base.join("eden").join("bis").join("user").join("save")));
    }

    #[cfg(target_os = "macos")]
    if let Ok(home) = std::env::var("HOME") {
        let base = PathBuf::from(&home);
        v.push(("Ryujinx", base.join("Library").join("Application Support").join("Ryujinx").join("bis").join("user").join("save")));
        v.push(("Eden",    base.join(".local").join("share").join("eden").join("bis").join("user").join("save")));
    }

    #[cfg(target_os = "linux")]
    if let Ok(home) = std::env::var("HOME") {
        let h = PathBuf::from(&home);
        let cfg  = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| h.join(".config"));
        let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| h.join(".local").join("share"));
        v.push(("Ryujinx", cfg .join("Ryujinx").join("bis").join("user").join("save")));
        v.push(("Eden",    data.join("eden").join("bis").join("user").join("save")));
    }

    v
}

fn is_valid_save_folder(dir: &Path) -> bool {
    let d0 = dir.join("0");
    let d1 = dir.join("1");
    (d0.is_dir() || d1.is_dir()) &&
    (find_ugc_dir(&d0).is_some() || find_ugc_dir(&d1).is_some())
}

/// Finds all Tomodachi Life save folders for known emulators.
/// Scans one level deep (root/{id}/) and two levels deep (root/{id}/{id}/)
/// because different Ryujinx/Eden versions use different layouts.
pub fn find_emulator_saves() -> Vec<FoundSave> {
    let mut found = Vec::new();
    for (name, root) in emulator_save_roots() {
        if !root.is_dir() { continue; }
        let Ok(lvl1) = std::fs::read_dir(&root) else { continue };
        for e1 in lvl1.flatten() {
            let p1 = e1.path();
            if !p1.is_dir() { continue; }

            // Level 1: root/{id}/ — check directly
            if is_valid_save_folder(&p1) {
                found.push(FoundSave {
                    emulator: name.to_string(),
                    path:     p1.to_string_lossy().into_owned(),
                });
                continue; // don't descend further if already matched
            }

            // Level 2: root/{id}/{id}/ — some builds nest one more level
            let Ok(lvl2) = std::fs::read_dir(&p1) else { continue };
            for e2 in lvl2.flatten() {
                let p2 = e2.path();
                if p2.is_dir() && is_valid_save_folder(&p2) {
                    found.push(FoundSave {
                        emulator: name.to_string(),
                        path:     p2.to_string_lossy().into_owned(),
                    });
                }
            }
        }
    }
    found
}

// ─── Save-folder slot scanning ────────────────────────────────────────────────

/// Finds the UGC sub-folder inside *parent* (e.g. inside "0" or "1") by
/// looking for the first directory that contains Ugc*.zs files.
fn find_ugc_dir(parent: &Path) -> Option<PathBuf> {
    if has_ugc_files(parent) { return Some(parent.to_path_buf()); }
    if let Ok(rd) = std::fs::read_dir(parent) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() && has_ugc_files(&p) { return Some(p); }
        }
    }
    None
}

fn has_ugc_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).map(|rd| {
        rd.flatten().any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("Ugc") && n.ends_with(".zs")
        })
    }).unwrap_or(false)
}

/// Finds thumb file paths for all IDs of *item_type* inside *dir*.
fn find_ids_in_dir(dir: &Path, item_type: &str) -> BTreeMap<u32, PathBuf> {
    let mut map = BTreeMap::new();
    let t = match tmpl(item_type) { Some(t) => t, None => return map };
    let (prefix, suffix) = match t.thumb.split_once("{id}") {
        Some(ps) => ps,
        None => return map,
    };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(prefix) && name.ends_with(suffix) {
                let mid = &name[prefix.len()..name.len() - suffix.len()];
                if let Ok(id) = mid.parse::<u32>() {
                    map.insert(id, entry.path());
                }
            }
        }
    }
    map
}

#[derive(Serialize, Clone)]
pub struct SlotInfo {
    pub id:      u32,
    pub preview: Option<String>,
    /// true = exists only in folder "1", not yet confirmed in folder "0"
    pub warning: bool,
}

/// Scans *save_dir*/0 and *save_dir*/1 for UGC textures of *item_type*.
pub fn scan_save_slots(save_dir: &Path, item_type: &str) -> Vec<SlotInfo> {
    let ugc0 = find_ugc_dir(&save_dir.join("0")).unwrap_or_else(|| save_dir.join("0"));
    let ugc1 = find_ugc_dir(&save_dir.join("1")).unwrap_or_else(|| save_dir.join("1"));
    let ids0 = find_ids_in_dir(&ugc0, item_type);
    let ids1 = find_ids_in_dir(&ugc1, item_type);

    let mut all_ids: std::collections::BTreeSet<u32> = Default::default();
    all_ids.extend(ids0.keys().copied());
    all_ids.extend(ids1.keys().copied());

    all_ids.into_iter().map(|id| {
        let in0 = ids0.contains_key(&id);
        let in1 = ids1.contains_key(&id);
        // Prefer the "1" thumbnail (newer), fall back to "0"
        let preview = ids1.get(&id).or_else(|| ids0.get(&id))
            .and_then(|p| std::fs::read(p).ok().and_then(|d| thumb_to_preview(&d)));
        SlotInfo { id, preview, warning: in1 && !in0 }
    }).collect()
}

// ─── Whole-folder backup ──────────────────────────────────────────────────────

fn copy_dir_all(src: &Path, dst: &Path, skip: &Path) -> std::io::Result<()> {
    if src == skip { return Ok(()); }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let sp = entry.path();
        if sp == skip { continue; }
        let dp = dst.join(entry.file_name());
        if sp.is_dir() { copy_dir_all(&sp, &dp, skip)?; } else { std::fs::copy(&sp, &dp)?; }
    }
    Ok(())
}

fn backup_save(save_dir: &Path) -> std::io::Result<()> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs()).unwrap_or(0);
    let backup_root = save_dir.join("_TTT_Backup").join(ts.to_string());
    let skip = save_dir.join("_TTT_Backup");
    // Back up both "0" and "1" sub-folders
    for sub in &["0", "1"] {
        let src = save_dir.join(sub);
        if src.exists() {
            copy_dir_all(&src, &backup_root.join(sub), &skip)?;
        }
    }
    Ok(())
}

// ─── Slot texture export ──────────────────────────────────────────────────────

/// Decodes the stored texture for a slot and returns it as a PNG byte vector.
/// Tries canvas.zs first (256×256 RGBA, lossless), then ugctex.zs (512×512 BC1).
pub fn export_slot_texture(
    save_dir: &Path,
    item_type: &str,
    item_id: u32,
) -> Result<Vec<u8>, String> {
    let t      = tmpl(item_type).ok_or_else(|| format!("Unknown item type: {item_type}"))?;
    let id_str = format!("{item_id:03}");

    // Try both sub-folders, prefer "0"
    for sub in &["0", "1"] {
        let sub_dir = save_dir.join(sub);
        let Some(ugc_dir) = find_ugc_dir(&sub_dir) else { continue };

        // ── Try canvas.zs (raw RGBA 256×256) ──────────────────────────────
        let canvas_path = ugc_dir.join(apply_id(t.canvas, &id_str));
        if canvas_path.exists() {
            if let Ok(zs) = std::fs::read(&canvas_path) {
                if let Ok(raw) = zstd::decode_all(std::io::Cursor::new(&zs)) {
                    let linear = nsw_unswizzle(&raw, 256, 256, 1, 1, 4, 4);
                    if let Some(img) = image::RgbaImage::from_raw(256, 256, linear) {
                        let dyn_img = image::DynamicImage::ImageRgba8(img);
                        let mut buf = std::io::Cursor::new(Vec::new());
                        dyn_img.write_to(&mut buf, image::ImageFormat::Png)
                            .map_err(|e| e.to_string())?;
                        return Ok(buf.into_inner());
                    }
                }
            }
        }

        // ── Fall back to ugctex.zs (BC1 512×512) ──────────────────────────
        let ugctex_path = ugc_dir.join(apply_id(t.ugctex, &id_str));
        if ugctex_path.exists() {
            if let Ok(zs) = std::fs::read(&ugctex_path) {
                if let Ok(raw) = zstd::decode_all(std::io::Cursor::new(&zs)) {
                    let linear = nsw_unswizzle(&raw, 512, 512, 4, 4, 8, 4);
                    let mut rgba = vec![0u8; 512 * 512 * 4];
                    squish::Format::Bc1.decompress(&linear, 512, 512, &mut rgba);
                    if let Some(img) = image::RgbaImage::from_raw(512, 512, rgba) {
                        let dyn_img = image::DynamicImage::ImageRgba8(img);
                        let mut buf = std::io::Cursor::new(Vec::new());
                        dyn_img.write_to(&mut buf, image::ImageFormat::Png)
                            .map_err(|e| e.to_string())?;
                        return Ok(buf.into_inner());
                    }
                }
            }
        }
    }

    Err("No texture file found for this slot".into())
}

// ─── Backup listing & restore ─────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct BackupInfo {
    pub timestamp: u64,
    pub label:     String,
    pub path:      String,
}

/// Formats a Unix timestamp (seconds) into a human-readable string,
/// e.g. "27 Apr 2026 — 18:08:45". Pure-Rust, no extra deps.
fn format_timestamp(ts: u64) -> String {
    let time_of_day = ts % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;

    // Civil date from days since epoch (Gregorian proleptic)
    let z     = ts as i64 / 86400 + 719_468;
    let era   = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe   = z - era * 146_097;
    let yoe   = (doe - doe/1_460 + doe/36_524 - doe/146_096) / 365;
    let doy   = doe - (365*yoe + yoe/4 - yoe/100);
    let mp    = (5*doy + 2) / 153;
    let day   = doy - (153*mp + 2)/5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year  = yoe + era * 400 + if month <= 2 { 1 } else { 0 };

    const MONTHS: &[&str] = &[
        "Jan","Feb","Mar","Apr","May","Jun",
        "Jul","Aug","Sep","Oct","Nov","Dec",
    ];
    let mn = MONTHS.get((month as usize).saturating_sub(1)).copied().unwrap_or("???");
    format!("{day} {mn} {year}  —  {:02}:{:02}:{:02}", h, m, s)
}

/// Returns all backups inside `save_dir/_TTT_Backup/`, newest first.
pub fn list_backups(save_dir: &Path) -> Vec<BackupInfo> {
    let backup_root = save_dir.join("_TTT_Backup");
    if !backup_root.exists() { return vec![]; }

    let mut backups: Vec<BackupInfo> = std::fs::read_dir(&backup_root)
        .ok().into_iter().flatten().flatten()
        .filter_map(|e| {
            let p = e.path();
            if !p.is_dir() { return None; }
            let ts: u64 = p.file_name()?.to_str()?.parse().ok()?;
            Some(BackupInfo {
                timestamp: ts,
                label: format_timestamp(ts),
                path:  p.to_string_lossy().into_owned(),
            })
        })
        .collect();
    backups.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    backups
}

/// Restores `backup_path/0` → `save_dir/0` and `backup_path/1` → `save_dir/1`.
/// Clears each destination sub-folder before copying so the result exactly
/// mirrors the backup (no leftover files from after the backup was taken).
pub fn restore_backup(save_dir: &Path, backup_path: &Path) -> Result<(), String> {
    let dummy = PathBuf::from("");
    for sub in &["0", "1"] {
        let src = backup_path.join(sub);
        let dst = save_dir.join(sub);
        if !src.exists() { continue; }
        if dst.exists() {
            std::fs::remove_dir_all(&dst)
                .map_err(|e| format!("Could not clear folder {sub}: {e}"))?;
        }
        copy_dir_all(&src, &dst, &dummy)
            .map_err(|e| format!("Could not restore folder {sub}: {e}"))?;
    }
    Ok(())
}

// ─── Public API ───────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ConvertResult {
    pub canvas:  String,
    pub ugctex:  String,
    pub thumb:   String,
    pub folders: Vec<String>,  // which sub-folders were written ("0", "1")
}

/// Converts *png_path* and writes the result into both *save_dir*/0 and
/// *save_dir*/1 (whichever exist). Backs up the whole save folder first.
pub fn convert(
    png_path: &Path,
    save_dir: &Path,
    item_type: &str,
    item_id: u32,
    on_progress: impl Fn(&str, f64),
) -> Result<ConvertResult, String> {
    let t = tmpl(item_type)
        .ok_or_else(|| format!("Unknown item type: {item_type}"))?;
    let id_str = format!("{item_id:03}");

    on_progress("Loading image…", 0.05);
    let img = image::open(png_path).map_err(|e| format!("Cannot open image: {e}"))?;

    on_progress("Building CANVAS (256×256 RGBA)…", 0.20);
    let canvas_raw = to_canvas(&img);

    on_progress("Building UGCTEX (512×512 BC1)…", 0.40);
    let ugctex_raw = to_ugctex(&img);

    on_progress("Building thumbnail (256×256 BC3)…", 0.58);
    let thumb_raw  = to_thumb(&img);

    on_progress("Compressing with ZSTD…", 0.72);
    let canvas_zs = zstd_compress(&canvas_raw).map_err(|e| e.to_string())?;
    let ugctex_zs = zstd_compress(&ugctex_raw).map_err(|e| e.to_string())?;
    let thumb_zs  = zstd_compress(&thumb_raw ).map_err(|e| e.to_string())?;

    on_progress("Backing up save folder…", 0.84);
    backup_save(save_dir).map_err(|e| format!("Backup failed: {e}"))?;

    on_progress("Writing files…", 0.94);
    let mut folders_written = Vec::new();
    let mut first_canvas = String::new();
    let mut first_ugctex = String::new();
    let mut first_thumb  = String::new();

    for sub in &["0", "1"] {
        let sub_dir = save_dir.join(sub);
        if !sub_dir.exists() { continue; }
        // Write into the UGC subfolder inside "0"/"1", auto-detected
        let out_dir = find_ugc_dir(&sub_dir).unwrap_or(sub_dir);

        let canvas_path = out_dir.join(apply_id(t.canvas, &id_str));
        let ugctex_path = out_dir.join(apply_id(t.ugctex, &id_str));
        let thumb_path  = out_dir.join(apply_id(t.thumb,  &id_str));

        std::fs::write(&canvas_path, &canvas_zs).map_err(|e| e.to_string())?;
        std::fs::write(&ugctex_path, &ugctex_zs).map_err(|e| e.to_string())?;
        std::fs::write(&thumb_path,  &thumb_zs ).map_err(|e| e.to_string())?;

        folders_written.push(sub.to_string());
        if first_canvas.is_empty() {
            first_canvas = canvas_path.to_string_lossy().into_owned();
            first_ugctex = ugctex_path.to_string_lossy().into_owned();
            first_thumb  = thumb_path .to_string_lossy().into_owned();
        }
    }

    if folders_written.is_empty() {
        return Err("No save sub-folders (0 or 1) found inside the chosen folder.".into());
    }

    on_progress("Done!", 1.0);
    Ok(ConvertResult {
        canvas:  first_canvas,
        ugctex:  first_ugctex,
        thumb:   first_thumb,
        folders: folders_written,
    })
}
