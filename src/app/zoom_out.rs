//! Filter the half-size map view from the original sprite pixels.

use super::{Framebuffer, Game};

/// Limit the reusable RGB(A) map buffer to 64 MiB.
const MAX_OUTPUT_PIXELS: u64 = 4 * 1024 * 1024;

pub(super) struct ZoomOut {
    enabled: bool,
    source: Option<Framebuffer>,
    pub(super) present_at_full_size: bool,
    output: Option<WideFrame>,
    ui_sample: Option<Framebuffer>,
}

struct WideFrame {
    frame: Framebuffer,
    sample: Framebuffer,
}

#[derive(Clone, Copy)]
pub(super) struct DrawingView {
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub zoom: i32,
}

impl ZoomOut {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            source: None,
            present_at_full_size: false,
            output: None,
            ui_sample: None,
        }
    }

    fn active(&self, (width, height): (u32, u32), zoom: i32) -> bool {
        self.enabled && zoom == -2 && u64::from(width) * u64::from(height) <= MAX_OUTPUT_PIXELS
    }

    pub(super) fn view(&self, size: (u32, u32), origin: (i32, i32), zoom: i32) -> DrawingView {
        if self.active(size, zoom) {
            DrawingView {
                origin: (origin.0.saturating_mul(2), origin.1.saturating_mul(2)),
                size: (size.0 * 2, size.1 * 2),
                zoom: 0,
            }
        } else {
            DrawingView { origin, size, zoom }
        }
    }

    pub(super) fn draw(
        &mut self,
        game: &Game,
        target: &mut Framebuffer,
        dirty: Option<&[[i32; 4]]>,
    ) {
        let size = (target.width, target.height);
        let zoom = game.camera.zoom;
        let view = self.view(size, game.camera.scroll, zoom);
        let (x, y) = view.origin;

        if !self.active(size, zoom) {
            self.source = None;
            match dirty {
                Some(rects) => game.staged.redraw(target, x, y, zoom, game.grid, rects),
                None => game.staged.draw(target, x, y, zoom, game.grid),
            }
            return;
        }

        let same_size = self
            .source
            .as_ref()
            .is_some_and(|frame| (frame.width, frame.height) == view.size);
        if !same_size {
            self.source = Some(Framebuffer::new(view.size.0, view.size.1));
        }
        let source = self.source.as_mut().unwrap();
        if let Some(rects) = dirty.filter(|_| same_size) {
            game.staged
                .redraw(source, x, y, view.zoom, game.grid, rects);
            for &rect in rects {
                downsample(source, target, rect.map(|edge| edge as usize));
            }
        } else {
            game.staged.draw(source, x, y, view.zoom, game.grid);
            downsample(
                source,
                target,
                [0, 0, source.width as usize, source.height as usize],
            );
        }
    }

    pub(super) fn compose(
        &mut self,
        has_game: bool,
        ui_scale: (usize, usize),
        mut paint: impl FnMut(&mut Framebuffer, &Framebuffer, (usize, usize), (usize, usize)),
    ) {
        let Some(source) = self
            .source
            .as_ref()
            .filter(|_| self.present_at_full_size && has_game)
        else {
            self.output = None;
            return;
        };
        let size = (source.width, source.height);
        let scale = (2 * ui_scale.0, ui_scale.1);
        if self
            .output
            .as_ref()
            .is_none_or(|wide| (wide.frame.width, wide.frame.height) != size)
        {
            self.output = Some(WideFrame {
                frame: Framebuffer::new(size.0, size.1),
                sample: sample_for(source, scale),
            });
        }
        let wide = self.output.as_mut().unwrap();
        composite(&mut wide.frame, &mut wide.sample, scale, |sample, phase| {
            paint(sample, source, phase, scale);
        });
    }

    pub(super) fn smooth_interface(
        &mut self,
        frame: &mut Framebuffer,
        map: &Framebuffer,
        scale: (usize, usize),
        paint: impl FnOnce(&mut Framebuffer) -> Option<(i32, i32, i32, i32)>,
    ) {
        let size = (
            layout_extent(frame.width, scale),
            layout_extent(frame.height, scale),
        );
        let layer = self
            .ui_sample
            .get_or_insert_with(|| sample_for(frame, scale));
        if (layer.width, layer.height) != size {
            *layer = sample_for(frame, scale);
        }
        let rect = paint(layer);
        filter_overlay(frame, map, layer, scale, rect);
        if !self.present_at_full_size {
            self.output = None;
            return;
        }

        let size = (frame.width * 2, frame.height * 2);
        let high_scale = (scale.0 * 2, scale.1);
        let wide = self.output.get_or_insert_with(|| WideFrame {
            frame: Framebuffer::new(size.0, size.1),
            sample: sample_for(layer, (1, 1)),
        });
        if (wide.frame.width, wide.frame.height) != size {
            wide.frame = Framebuffer::new(size.0, size.1);
        }
        filter_overlay(
            &mut wide.frame,
            self.source.as_ref().unwrap_or(map),
            layer,
            high_scale,
            rect,
        );
    }

    #[cfg(test)]
    pub(super) fn compose_ui(
        &mut self,
        frame: &mut Framebuffer,
        scale: (usize, usize),
        paint: impl FnMut(&mut Framebuffer, (usize, usize)),
    ) {
        let size = (
            layout_extent(frame.width, scale),
            layout_extent(frame.height, scale),
        );
        let sample = self
            .ui_sample
            .get_or_insert_with(|| sample_for(frame, scale));
        if (sample.width, sample.height) != size {
            *sample = sample_for(frame, scale);
        }
        composite(frame, sample, scale, paint);
    }

    pub(super) fn presented(&self, has_game: bool) -> Option<&Framebuffer> {
        if !self.present_at_full_size || !has_game {
            return None;
        }
        self.output
            .as_ref()
            .map(|wide| &wide.frame)
            .or(self.source.as_ref())
    }
}

/// Filter premultiplied HUD colors so transparent edges never blend neighboring world pixels.
fn filter_overlay(
    out: &mut Framebuffer,
    map: &Framebuffer,
    layer: &Framebuffer,
    (numerator, denominator): (usize, usize),
    rect: Option<(i32, i32, i32, i32)>,
) {
    let axis = |length: u32, limit: u32| {
        (0..length)
            .map(|at| {
                // Sample at pixel centers, using fixed-point weights for the inner loop.
                let center =
                    ((2 * at as i64 + 1) * denominator as i64 * 128) / numerator as i64 - 128;
                let left = center.div_euclid(256);
                let weight = center.rem_euclid(256) as u32;
                let clamp = |at: i64| at.clamp(0, i64::from(limit) - 1) as usize;
                (clamp(left), clamp(left + 1), weight)
            })
            .collect::<Vec<_>>()
    };
    let xs = axis(out.width, layer.width);
    let ys = axis(out.height, layer.height);
    let map_scale = (out.width / map.width).max(1) as usize;
    let map_pixels = map.pixels.as_chunks::<4>().0;
    let ui = layer.pixels.as_chunks::<4>().0;
    let output = out.pixels.as_chunks_mut::<4>().0;
    for (y, &(top, bottom, fy)) in ys.iter().enumerate() {
        let row = (y / map_scale) * map.width as usize;
        let uy = (y * denominator / numerator) as i32;
        let bounds = rect.filter(|&(_, t, _, h)| uy >= t && uy < t.saturating_add(h));
        let top = top * layer.width as usize;
        let bottom = bottom * layer.width as usize;
        for (x, &(left, right, fx)) in xs.iter().enumerate() {
            let ux = (x * denominator / numerator) as i32;
            let base = if bounds.is_some_and(|(l, _, w, _)| ux >= l && ux < l.saturating_add(w)) {
                map_pixels[row + x / map_scale]
            } else {
                [0, 0, 0, 255]
            };
            let dst = &mut output[y * xs.len() + x];
            let taps = [
                ui[top + left],
                ui[top + right],
                ui[bottom + left],
                ui[bottom + right],
            ];
            if taps.iter().all(|p| p[3] == 0) {
                *dst = base;
                continue;
            }
            let weights = [
                (256 - fx) * (256 - fy),
                fx * (256 - fy),
                (256 - fx) * fy,
                fx * fy,
            ];
            let coverage = taps.map(|p| u32::from(p[3]));
            let factors = std::array::from_fn::<_, 4, _>(|i| weights[i] * coverage[i]);
            const FULL: u32 = 65536 * 255;
            let behind = FULL - factors.iter().sum::<u32>();
            for c in 0..3 {
                let fg = (0..4)
                    .map(|i| factors[i] * u32::from(taps[i][c]))
                    .sum::<u32>();
                dst[c] = ((fg + behind * u32::from(base[c]) + FULL / 2) / FULL) as u8;
            }
            dst[3] = 255;
        }
    }
}

fn layout_extent(extent: u32, (numerator, denominator): (usize, usize)) -> u32 {
    if extent == 0 {
        return 0;
    }
    (((extent - 1) as usize * denominator) / numerator + 1) as u32
}

fn sample_for(frame: &Framebuffer, scale: (usize, usize)) -> Framebuffer {
    Framebuffer::new(
        layout_extent(frame.width, scale),
        layout_extent(frame.height, scale),
    )
}

/// Paint against each map sample before interleaving them, so translucent UI preserves
/// the original world pixels underneath it.
fn composite(
    frame: &mut Framebuffer,
    sample: &mut Framebuffer,
    scale: (usize, usize),
    mut paint: impl FnMut(&mut Framebuffer, (usize, usize)),
) {
    let rect = (0, 0, sample.width as i32, sample.height as i32);
    let phases = scale.0.div_ceil(scale.1);
    for y in 0..phases {
        for x in 0..phases {
            paint(sample, (x, y));
            copy_sample(sample, frame, rect, (x, y), scale, false);
        }
    }
}

pub(super) fn copy_sample(
    from: &Framebuffer,
    to: &mut Framebuffer,
    (x, y, w, h): (i32, i32, i32, i32),
    (sample_x, sample_y): (usize, usize),
    scale: (usize, usize),
    to_logical: bool,
) {
    let (large, small) = if to_logical {
        ((from.width, from.height), (to.width, to.height))
    } else {
        ((to.width, to.height), (from.width, from.height))
    };
    let width = layout_extent(large.0, scale).min(small.0) as i32;
    let height = layout_extent(large.1, scale).min(small.1) as i32;
    let edge = |at: usize| (at * scale.0).div_ceil(scale.1);
    let left = x.clamp(0, width) as usize;
    let right = x.saturating_add(w).clamp(0, width) as usize;
    let top = y.clamp(0, height) as usize;
    let bottom = y.saturating_add(h).clamp(0, height) as usize;
    let input = from.pixels.as_chunks::<4>().0;
    let output = to.pixels.as_chunks_mut::<4>().0;
    for row in top..bottom {
        let y = edge(row) + sample_y;
        if y >= edge(row + 1).min(large.1 as usize) {
            continue;
        }
        let large_row = y * large.0 as usize;
        let small_row = row * small.0 as usize;
        for col in left..right {
            let x = edge(col) + sample_x;
            if x >= edge(col + 1).min(large.0 as usize) {
                continue;
            }
            let large_pixel = large_row + x;
            let small_pixel = small_row + col;
            if to_logical {
                output[small_pixel] = input[large_pixel];
            } else {
                output[large_pixel] = input[small_pixel];
            }
        }
    }
}

/// Update every destination pixel touched by the source rectangle, including partially covered
/// 2×2 blocks along its edges. The rendered map is opaque.
fn downsample(
    source: &Framebuffer,
    target: &mut Framebuffer,
    [left, top, right, bottom]: [usize; 4],
) {
    let width = target.width as usize;
    let stride = source.width as usize;
    let input = source.pixels.as_chunks::<4>().0;
    let output = target.pixels.as_chunks_mut::<4>().0;
    for y in top / 2..bottom.div_ceil(2) {
        for x in left / 2..right.div_ceil(2) {
            let i = y * 2 * stride + x * 2;
            let a = input[i];
            let b = input[i + 1];
            let c = input[i + stride];
            let d = input[i + stride + 1];
            output[y * width + x] = [
                average(a[0], b[0], c[0], d[0]),
                average(a[1], b[1], c[1], d[1]),
                average(a[2], b[2], c[2], d[2]),
                255,
            ];
        }
    }
}

fn average(a: u8, b: u8, c: u8, d: u8) -> u8 {
    ((u16::from(a) + u16::from(b) + u16::from(c) + u16::from(d) + 2) / 4) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thin_sprite_remains_visible_and_odd_dirty_edges_refresh_its_pixel() {
        let mut source = Framebuffer::new(4, 2);
        let mut target = Framebuffer::new(2, 1);
        target.fill([17, 19, 23, 255]);
        source.pixels[4..8].copy_from_slice(&[255, 128, 0, 255]);

        downsample(&source, &mut target, [1, 0, 2, 1]);

        assert_eq!(target.pixels, [64, 32, 0, 255, 17, 19, 23, 255]);
    }

    #[test]
    fn wide_presentation_keeps_all_map_pixels_and_scales_the_interface() {
        let mut source = Framebuffer::new(4, 2);
        source.fill_test_pattern();
        let world = source.pixels.clone();
        let mut zoom = ZoomOut::new(true);
        zoom.source = Some(source);
        zoom.present_at_full_size = true;
        zoom.compose(true, (1, 1), |frame, map, phase, scale| {
            copy_sample(map, frame, (0, 0, 2, 1), phase, scale, true);
            frame.pixels[4..8].copy_from_slice(&[80, 20, 10, 255]);
        });

        let wide = zoom.presented(true).unwrap();
        assert_eq!((wide.width, wide.height), (4, 2));
        for row in [0, 16] {
            assert_eq!(&wide.pixels[row..row + 8], &world[row..row + 8]);
            assert_eq!(
                &wide.pixels[row + 8..row + 16],
                &[80, 20, 10, 255, 80, 20, 10, 255]
            );
        }
        assert!(zoom.presented(false).is_none());
    }

    #[test]
    fn doubled_ui_keeps_world_pixels_at_the_edge_of_an_odd_viewport() {
        let mut map = Framebuffer::new(5, 3);
        map.fill_test_pattern();
        let mut result = Framebuffer::new(5, 3);
        let mut zoom = ZoomOut::new(true);
        zoom.compose_ui(&mut result, (2, 1), |sample, phase| {
            copy_sample(&map, sample, (0, 0, 3, 2), phase, (2, 1), true);
            sample.pixels[..4].copy_from_slice(&[80, 20, 10, 255]);
        });
        for y in 0..3 {
            for x in 0..5 {
                let pixel = (y * 5 + x) * 4;
                let expected = if x < 2 && y < 2 {
                    &[80, 20, 10, 255]
                } else {
                    &map.pixels[pixel..pixel + 4]
                };
                assert_eq!(&result.pixels[pixel..pixel + 4], expected);
            }
        }
    }

    #[test]
    fn half_zoom_uses_a_double_size_normal_view_only_when_enabled() {
        let view = ZoomOut::new(true).view((1024, 768), (-7, 9), -2);
        assert_eq!(
            (view.size, view.origin, view.zoom),
            ((2048, 1536), (-14, 18), 0)
        );

        let view = ZoomOut::new(false).view((1024, 768), (-7, 9), -2);
        assert_eq!(
            (view.size, view.origin, view.zoom),
            ((1024, 768), (-7, 9), -2)
        );

        let view = ZoomOut::new(true).view((3000, 2000), (0, 0), -2);
        assert_eq!(view.zoom, -2);
    }
}
