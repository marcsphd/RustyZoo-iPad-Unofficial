//! Filter the half-size map view from the original sprite pixels.

use super::{Framebuffer, Game};

/// Limit the reusable RGB(A) map buffer to 64 MiB.
const MAX_OUTPUT_PIXELS: u64 = 4 * 1024 * 1024;

pub(super) struct ZoomOut {
    enabled: bool,
    source: Option<Framebuffer>,
    pub(super) present_at_full_size: bool,
    output: Option<WideFrame>,
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
        mut paint: impl FnMut(&mut Framebuffer, &Framebuffer, (usize, usize)),
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
        if self
            .output
            .as_ref()
            .is_none_or(|wide| (wide.frame.width, wide.frame.height) != size)
        {
            self.output = Some(WideFrame {
                frame: Framebuffer::new(size.0, size.1),
                sample: Framebuffer::new(size.0 / 2, size.1 / 2),
            });
        }
        let wide = self.output.as_mut().unwrap();
        // Each logical UI pixel covers a 2×2 block. Composite the same interface over
        // each of the four map samples to preserve their colors, including under
        // partially transparent UI artwork and text.
        for y in 0..2 {
            for x in 0..2 {
                paint(&mut wide.sample, source, (x, y));
                copy_sample(
                    &wide.sample,
                    &mut wide.frame,
                    (0, 0, wide.sample.width as i32, wide.sample.height as i32),
                    (x, y),
                    false,
                );
            }
        }
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

/// Copies between logical UI pixels and one of the four positions in their 2×2 blocks.
pub(super) fn copy_sample(
    from: &Framebuffer,
    to: &mut Framebuffer,
    (x, y, w, h): (i32, i32, i32, i32),
    (sample_x, sample_y): (usize, usize),
    to_logical: bool,
) {
    let (from_stride, to_stride) = (from.width as usize, to.width as usize);
    let (width, height) = if to_logical {
        (to.width, to.height)
    } else {
        (from.width, from.height)
    };
    let left = x.clamp(0, width as i32) as usize;
    let right = x.saturating_add(w).clamp(0, width as i32) as usize;
    let top = y.clamp(0, height as i32) as usize;
    let bottom = y.saturating_add(h).clamp(0, height as i32) as usize;
    let input = from.pixels.as_chunks::<4>().0;
    let output = to.pixels.as_chunks_mut::<4>().0;
    for row in top..bottom {
        if to_logical {
            let src = (row * 2 + sample_y) * from_stride + left * 2 + sample_x;
            let dst = row * to_stride + left;
            for (col, pixel) in output[dst..dst + right.saturating_sub(left)]
                .iter_mut()
                .enumerate()
            {
                *pixel = input[src + col * 2];
            }
        } else {
            let src = row * from_stride + left;
            let dst = (row * 2 + sample_y) * to_stride + left * 2 + sample_x;
            for (col, pixel) in input[src..src + right.saturating_sub(left)]
                .iter()
                .enumerate()
            {
                output[dst + col * 2] = *pixel;
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
        zoom.compose(true, |frame, map, phase| {
            copy_sample(map, frame, (0, 0, 2, 1), phase, true);
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
