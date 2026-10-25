//! Turning a Gaea build into files an Unreal landscape imports without guesswork.
//!
//! Gaea builds at powers of two (1024, 4096, 8192), but an Unreal landscape is laid out in
//! components of sections of quads, so the vertex counts it accepts are `quads * sections *
//! components + 1` - 8129, 8161, 4033 and so on. Importing a 8192 heightmap makes Unreal pad or
//! crop it, which shifts every feature by a few metres against the masks. The heightmap is
//! therefore resampled to an accepted size here, the masks with it, and the scale that makes the
//! landscape come out at the intended size in metres is written beside them.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Quads per section Unreal offers in the landscape creation panel.
const QUADS_PER_SECTION: [usize; 6] = [7, 15, 31, 63, 127, 255];
/// Sections per component Unreal offers (1x1 or 2x2).
const SECTIONS_PER_COMPONENT: [usize; 2] = [1, 2];
/// More components per side than this makes the editor panel refuse the layout.
const MAX_COMPONENTS_PER_SIDE: usize = 32;

/// One way Unreal can lay out a landscape of a given vertex count.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct LandscapeLayout {
    pub quads_per_section: usize,
    pub sections_per_component: usize,
    pub components_per_side: usize,
}

/// Every layout that yields exactly `size` vertices per side, largest sections first: fewer,
/// larger components are cheaper to render and stream.
pub fn layouts_for(size: usize) -> Vec<LandscapeLayout> {
    let mut layouts = Vec::new();
    if size < 2 {
        return layouts;
    }
    let quads = size - 1;
    for &q in QUADS_PER_SECTION.iter().rev() {
        for &s in SECTIONS_PER_COMPONENT.iter().rev() {
            let per_component = q * s;
            if quads % per_component == 0 {
                let components = quads / per_component;
                if (1..=MAX_COMPONENTS_PER_SIDE).contains(&components) {
                    layouts.push(LandscapeLayout {
                        quads_per_section: q,
                        sections_per_component: s,
                        components_per_side: components,
                    });
                }
            }
        }
    }
    layouts
}

/// The accepted size closest to `wanted`, preferring the larger on a tie.
pub fn nearest_valid_size(wanted: usize) -> usize {
    let mut best = 0usize;
    for &q in &QUADS_PER_SECTION {
        for &s in &SECTIONS_PER_COMPONENT {
            for c in 1..=MAX_COMPONENTS_PER_SIDE {
                let size = q * s * c + 1;
                let better = best == 0
                    || size.abs_diff(wanted) < best.abs_diff(wanted)
                    || (size.abs_diff(wanted) == best.abs_diff(wanted) && size > best);
                if better {
                    best = size;
                }
            }
        }
    }
    best
}

/// Resample a square field to `target` samples per side, bilinearly, keeping the corners on the
/// corners so a feature at the edge of the build stays at the edge of the landscape.
pub fn resample(samples: &[f64], size: usize, target: usize) -> Vec<f64> {
    if size == target {
        return samples.to_vec();
    }
    let scale = if target > 1 {
        (size - 1) as f64 / (target - 1) as f64
    } else {
        0.0
    };
    let mut out = Vec::with_capacity(target * target);
    for row in 0..target {
        let y = row as f64 * scale;
        let y0 = (y.floor() as usize).min(size - 1);
        let y1 = (y0 + 1).min(size - 1);
        let fy = y - y0 as f64;
        for col in 0..target {
            let x = col as f64 * scale;
            let x0 = (x.floor() as usize).min(size - 1);
            let x1 = (x0 + 1).min(size - 1);
            let fx = x - x0 as f64;
            let top = samples[y0 * size + x0] * (1.0 - fx) + samples[y0 * size + x1] * fx;
            let bottom = samples[y1 * size + x0] * (1.0 - fx) + samples[y1 * size + x1] * fx;
            out.push(top * (1.0 - fy) + bottom * fy);
        }
    }
    out
}

/// Cut a field into `tiles` x `tiles` pieces that share their border row and column, the way
/// Unreal's tiled heightmap import expects (`_x0_y0` is the north-west tile).
pub fn split_tiles(samples: &[f64], size: usize, tiles: usize) -> Result<Vec<(usize, usize, Vec<f64>)>, String> {
    if tiles <= 1 {
        return Ok(vec![(0, 0, samples.to_vec())]);
    }
    let quads = size - 1;
    if quads % tiles != 0 {
        return Err(format!(
            "{size} vertices cannot be split into {tiles}x{tiles} tiles that share edges; \
             {quads} quads must divide by {tiles}"
        ));
    }
    let step = quads / tiles;
    let tile_size = step + 1;
    let mut out = Vec::with_capacity(tiles * tiles);
    for ty in 0..tiles {
        for tx in 0..tiles {
            let mut tile = Vec::with_capacity(tile_size * tile_size);
            for row in 0..tile_size {
                let start = (ty * step + row) * size + tx * step;
                tile.extend_from_slice(&samples[start..start + tile_size]);
            }
            out.push((tx, ty, tile));
        }
    }
    Ok(out)
}

/// Write a 16-bit greyscale PNG from samples in 0..1.
pub fn write_png16(path: &Path, samples: &[f64], size: usize) -> Result<(), String> {
    let mut data = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16;
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_png(path, &data, size, png::BitDepth::Sixteen)
}

/// Write an 8-bit greyscale PNG from samples in 0..1. Unreal reads layer weights as 8-bit.
pub fn write_png8(path: &Path, samples: &[f64], size: usize) -> Result<(), String> {
    let data: Vec<u8> = samples
        .iter()
        .map(|s| (s.clamp(0.0, 1.0) * f64::from(u8::MAX)).round() as u8)
        .collect();
    write_png(path, &data, size, png::BitDepth::Eight)
}

/// Write little-endian 16-bit raw, Unreal's `.r16`.
pub fn write_r16(path: &Path, samples: &[f64]) -> Result<(), String> {
    let mut data = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let v = (s.clamp(0.0, 1.0) * f64::from(u16::MAX)).round() as u16;
        data.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, data).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

fn write_png(path: &Path, data: &[u8], size: usize, depth: png::BitDepth) -> Result<(), String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
    let side = u32::try_from(size).map_err(|_| format!("{size} is too large for a PNG"))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), side, side);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(depth);
    let mut writer = encoder
        .write_header()
        .map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    writer
        .write_image_data(data)
        .map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// The transform that makes an imported landscape match the build in metres.
#[derive(Serialize, Debug)]
pub struct LandscapeScale {
    /// Landscape actor scale X and Y: centimetres between neighbouring vertices.
    pub xy: f64,
    /// Landscape actor scale Z. Unreal maps the full 16-bit range onto 512 x Z/100 metres.
    pub z: f64,
    /// Actor location Z in centimetres that puts the lowest sample at world height 0. Unreal
    /// centres the 16-bit range on the actor, so without it half the terrain sits below zero.
    pub location_z: f64,
}

/// Scale for a landscape of `size` vertices spanning `world_m` metres with `height_m` metres
/// between its lowest and highest sample.
pub fn landscape_scale(size: usize, world_m: f64, height_m: f64) -> LandscapeScale {
    let z = height_m * 100.0 / 512.0;
    LandscapeScale {
        xy: world_m * 100.0 / (size.max(2) - 1) as f64,
        z,
        location_z: 256.0 * z,
    }
}

/// What an export wrote and how to import it.
#[derive(Serialize)]
pub struct ExportManifest {
    pub name: String,
    pub source: String,
    pub source_resolution: usize,
    pub resolution: usize,
    pub world_size_m: f64,
    pub height_range_m: f64,
    pub metres_per_vertex: f64,
    pub layouts: Vec<LandscapeLayout>,
    pub scale: LandscapeScale,
    pub tiles_per_side: usize,
    pub heightmaps: Vec<String>,
    pub masks: Vec<MaskExport>,
    pub import_notes: Vec<String>,
}

#[derive(Serialize)]
pub struct MaskExport {
    pub name: String,
    pub source: String,
    pub files: Vec<String>,
    /// Mean coverage, 0..1, after resampling. Near 0 means the layer will barely show.
    pub mean: f64,
}

/// Resample one field and write it, tiled or whole, returning the files written.
pub fn write_field(
    dir: &Path,
    stem: &str,
    field: &[f64],
    size: usize,
    tiles: usize,
    writer: &dyn Fn(&Path, &[f64], usize) -> Result<(), String>,
    extension: &str,
) -> Result<Vec<String>, String> {
    let pieces = split_tiles(field, size, tiles)?;
    let tile_size = if tiles <= 1 { size } else { (size - 1) / tiles + 1 };
    let mut files = Vec::new();
    for (tx, ty, piece) in pieces {
        let file: PathBuf = if tiles <= 1 {
            dir.join(format!("{stem}.{extension}"))
        } else {
            dir.join(format!("{stem}_x{tx}_y{ty}.{extension}"))
        };
        writer(&file, &piece, tile_size)?;
        files.push(file.to_string_lossy().to_string());
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_unreal_sizes_have_layouts() {
        for size in [8129, 8161, 4033, 2017, 1009, 505] {
            assert!(!layouts_for(size).is_empty(), "{size} should be importable");
        }
        // 8129 = 127 quads x 2 sections x 32 components + 1
        assert!(layouts_for(8129).contains(&LandscapeLayout {
            quads_per_section: 127,
            sections_per_component: 2,
            components_per_side: 32,
        }));
    }

    #[test]
    fn powers_of_two_are_not_landscape_sizes() {
        assert!(layouts_for(8192).is_empty());
        assert!(layouts_for(4096).is_empty());
        assert_eq!(nearest_valid_size(8192), 8161);
        assert_eq!(nearest_valid_size(4096), 4081);
    }

    #[test]
    fn resampling_keeps_corners() {
        let size = 5;
        let field: Vec<f64> = (0..size * size).map(|i| i as f64 / 24.0).collect();
        let out = resample(&field, size, 9);
        assert_eq!(out.len(), 81);
        assert!((out[0] - field[0]).abs() < 1e-12);
        assert!((out[80] - field[24]).abs() < 1e-12);
        assert!((out[8] - field[4]).abs() < 1e-12);
    }

    #[test]
    fn tiles_share_their_edges() {
        let size = 9;
        let field: Vec<f64> = (0..size * size).map(|i| i as f64).collect();
        let tiles = split_tiles(&field, size, 2).unwrap();
        assert_eq!(tiles.len(), 4);
        let (_, _, west) = &tiles[0];
        let (_, _, east) = &tiles[1];
        // last column of the west tile is the first column of the east tile
        for row in 0..5 {
            assert_eq!(west[row * 5 + 4], east[row * 5]);
        }
        assert!(split_tiles(&field, size, 3).is_err());
    }

    #[test]
    fn scale_matches_world_size() {
        let scale = landscape_scale(8129, 8000.0, 600.0);
        // 8 km over 8128 quads is a little under a metre per vertex
        assert!((scale.xy - 98.425).abs() < 0.01);
        // 600 m over the 512 m Unreal maps at scale 100
        assert!((scale.z - 117.1875).abs() < 1e-9);
        assert!((scale.location_z - 30000.0).abs() < 1e-6);
    }
}
