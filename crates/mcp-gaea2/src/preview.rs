//! A picture of the terrain a build produced, small enough to hand back to the caller.
//!
//! A heightfield read as a greyscale image is a fog of mid-greys: a crater shows up as a slightly
//! darker disc and a cliff barely at all. Shading it by the light falling across its slopes turns
//! the same data into relief anyone can read at a glance, which is what an agent iterating on a
//! graph needs to see after each build.

use crate::terrain::Heightfield;

/// Light and scale for a hillshade.
pub struct ShadeOptions {
    /// Metres from one edge of the map to the other.
    pub metres_across: f64,
    /// Metres between the lowest and highest sample.
    pub metres_up: f64,
    /// Compass direction the light comes from, degrees clockwise from north (up).
    pub azimuth_degrees: f64,
    /// Height of the light above the horizon, degrees.
    pub altitude_degrees: f64,
    /// Longest side of the picture in pixels; larger fields are averaged down to this.
    pub max_size: usize,
}

/// Average a square field down to at most `max_size` per side.
pub fn downsample(samples: &[f64], size: usize, max_size: usize) -> (Vec<f64>, usize) {
    if size <= max_size || max_size == 0 {
        return (samples.to_vec(), size);
    }
    let out_size = max_size;
    let mut out = vec![0.0; out_size * out_size];
    for row in 0..out_size {
        let y0 = row * size / out_size;
        let y1 = ((row + 1) * size / out_size).max(y0 + 1);
        for col in 0..out_size {
            let x0 = col * size / out_size;
            let x1 = ((col + 1) * size / out_size).max(x0 + 1);
            let mut sum = 0.0;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += samples[y * size + x];
                }
            }
            out[row * out_size + col] = sum / ((y1 - y0) * (x1 - x0)) as f64;
        }
    }
    (out, out_size)
}

/// Lambertian hillshade, 0..1, of a field of heights in 0..1.
pub fn hillshade(samples: &[f64], size: usize, options: &ShadeOptions) -> Vec<f64> {
    let step = options.metres_across / size.max(2) as f64;
    let azimuth = options.azimuth_degrees.to_radians();
    let altitude = options.altitude_degrees.to_radians();
    // Light direction: x east, y south (row order), z up.
    let light = (
        altitude.cos() * azimuth.sin(),
        -altitude.cos() * azimuth.cos(),
        altitude.sin(),
    );
    let at = |row: usize, col: usize| samples[row * size + col] * options.metres_up;

    let mut shade = vec![0.0; size * size];
    for row in 0..size {
        let up = row.saturating_sub(1);
        let down = (row + 1).min(size - 1);
        for col in 0..size {
            let left = col.saturating_sub(1);
            let right = (col + 1).min(size - 1);
            let dz_dx = (at(row, right) - at(row, left)) / ((right - left).max(1) as f64 * step);
            let dz_dy = (at(down, col) - at(up, col)) / ((down - up).max(1) as f64 * step);
            // Surface normal of z = f(x, y) is (-dz/dx, -dz/dy, 1), normalised.
            let length = (dz_dx * dz_dx + dz_dy * dz_dy + 1.0).sqrt();
            let lit = (-dz_dx * light.0 - dz_dy * light.1 + light.2) / length;
            shade[row * size + col] = lit.clamp(0.0, 1.0);
        }
    }
    shade
}

/// Encode 0..1 samples as an 8-bit greyscale PNG in memory.
pub fn encode_png8(samples: &[f64], size: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    {
        let side = u32::try_from(size).map_err(|_| format!("{size} is too large for a PNG"))?;
        let mut encoder = png::Encoder::new(&mut bytes, side, side);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("Cannot encode the preview: {e}"))?;
        let data: Vec<u8> = samples
            .iter()
            .map(|s| (s.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect();
        writer
            .write_image_data(&data)
            .map_err(|e| format!("Cannot encode the preview: {e}"))?;
    }
    Ok(bytes)
}

/// Shade a loaded heightfield at preview size and return the PNG bytes and the size used.
pub fn render(field: &Heightfield, options: &ShadeOptions) -> Result<(Vec<u8>, usize), String> {
    let (small, size) = downsample(&field.samples, field.size, options.max_size);
    let shade = hillshade(&small, size, options);
    Ok((encode_png8(&shade, size)?, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> ShadeOptions {
        ShadeOptions {
            metres_across: 1000.0,
            metres_up: 100.0,
            azimuth_degrees: 315.0,
            altitude_degrees: 45.0,
            max_size: 64,
        }
    }

    #[test]
    fn flat_ground_is_lit_evenly() {
        let shade = hillshade(&vec![0.5; 16 * 16], 16, &options());
        let first = shade[0];
        assert!(shade.iter().all(|s| (s - first).abs() < 1e-12));
        // flat ground under a 45 degree sun reflects sin(45)
        assert!((first - 45f64.to_radians().sin()).abs() < 1e-9);
    }

    #[test]
    fn slopes_facing_the_light_are_brighter() {
        // Light from the north-west: ground falling away towards the north-west (rising to the
        // south-east) has its face turned to the light.
        let size = 32;
        let away: Vec<f64> = (0..size * size)
            .map(|i| 1.0 - ((i / size) + (i % size)) as f64 / (2 * size) as f64)
            .collect();
        let toward: Vec<f64> = away.iter().map(|v| 1.0 - v).collect();
        let lit = hillshade(&toward, size, &options())[size * 16 + 16];
        let dark = hillshade(&away, size, &options())[size * 16 + 16];
        assert!(lit > dark, "lit {lit} should exceed dark {dark}");
    }

    #[test]
    fn downsampling_averages_blocks() {
        let size = 4;
        let field: Vec<f64> = (0..16).map(|i| i as f64).collect();
        let (out, out_size) = downsample(&field, size, 2);
        assert_eq!(out_size, 2);
        // top-left block is 0, 1, 4, 5
        assert!((out[0] - 2.5).abs() < 1e-12);
    }

    #[test]
    fn preview_is_a_png() {
        let bytes = encode_png8(&vec![0.5; 8 * 8], 8).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
