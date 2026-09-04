//! Aspect-ratio-aware tile packing for the monitor wall (issue #20 item M4).
//!
//! D-2 (`.agents/issue-20/DESIGN-monitor-mode.md`) chose packing that
//! accounts for each camera's own native aspect ratio over a simple
//! `ceil(sqrt(n))` equal-size grid, so a fleet with genuinely different
//! aspect ratios (a portrait-mounted camera alongside landscape ones, say)
//! gets tile shapes proportional to each camera's own resolution rather
//! than forcing every camera into an identically shaped cell.
//!
//! # Algorithm
//!
//! This is a row-based bin-pack, the same family as the "justified" layouts
//! photo galleries use, adapted to fill a *fixed* box (a viewport) exactly
//! rather than flow to whatever height the content happens to need:
//!
//! 1. Try every row count from 1 to N. For a candidate row count, walk the
//!    cameras in input order, closing the current row once its accumulated
//!    aspect ratio reaches its fair share of the total
//!    (`total_aspect_ratio / row_count`) -- so a row holding narrower
//!    (portrait) cameras naturally ends up with more of them than a row of
//!    wide (landscape) ones.
//! 2. For each candidate partition, compute each row's own "natural"
//!    height: the height at which the row's cameras, each keeping its own
//!    aspect ratio, sum to exactly the viewport's width. Summing every
//!    row's natural height gives the partition's natural total height.
//! 3. Pick the candidate whose natural total height is closest to the
//!    viewport's actual height, then scale every row's height by the same
//!    factor to make the total match exactly.
//!
//! That last, uniform scale is the one place this algorithm trades away
//! perfect aspect-ratio fidelity for exact coverage: each tile's width
//! stays sized to fill the viewport's width exactly (the harder constraint
//! to give up), but scaling only the row's height by the correction factor
//! changes that tile's own box aspect ratio by the same factor. Choosing
//! the row count that minimizes the correction's distance from 1 keeps the
//! effect small, and it lands identically on every tile rather than
//! disproportionately on any one camera. `monitor.rs`'s own player element
//! absorbs whatever remains with `object-fit: contain` on the actual
//! video/canvas, so the mismatch surfaces as a little letterboxing inside
//! the box rather than a stretched picture: the tile's outer rectangle may
//! not be pixel-exact to the camera's native ratio, but the image it shows
//! always is.
//!
//! A wider-aspect camera still ends up proportionally wider than a
//! narrower one sharing its row (and, across the whole wall, occupies
//! proportionally more total area) -- the property D-2 actually asked for.
//! Nothing about the row-count search or the final scale changes that
//! ordering.

/// One tile's placement within the packed viewport, in the same units
/// (typically CSS pixels) as the viewport passed to [`pack_tiles`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct TileRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Packs each camera's native resolution (`width`/`height`, in any common
/// unit -- pixels, as Frigate reports them) into a non-overlapping
/// rectangle, sized so every rectangle together exactly tiles a
/// `viewport_width` x `viewport_height` box with no leftover margin.
///
/// Returns one rectangle per input, in input order. Returns an empty
/// vector for an empty input or a non-positive viewport dimension.
#[must_use]
pub(crate) fn pack_tiles(
    cameras: &[(u32, u32)],
    viewport_width: f64,
    viewport_height: f64,
) -> Vec<TileRect> {
    if cameras.is_empty() || viewport_width <= 0.0 || viewport_height <= 0.0 {
        return Vec::new();
    }

    let aspect_ratios: Vec<f64> = cameras
        .iter()
        .map(|&(width, height)| f64::from(width) / f64::from(height).max(1.0))
        .collect();

    let (rows, row_natural_heights, natural_total_height) = (1..=aspect_ratios.len())
        .map(|row_count| {
            let rows = partition_into_rows(&aspect_ratios, row_count);
            let row_natural_heights = natural_row_heights(&rows, &aspect_ratios, viewport_width);
            let natural_total_height = row_natural_heights.iter().sum::<f64>();
            (rows, row_natural_heights, natural_total_height)
        })
        .min_by(|(.., left), (.., right)| {
            (left - viewport_height)
                .abs()
                .total_cmp(&(right - viewport_height).abs())
        })
        .expect("at least one row count (1..=n) is tried for a non-empty camera list");

    let vertical_scale = viewport_height / natural_total_height;

    let mut placements = vec![TileRect::default(); aspect_ratios.len()];
    let mut y = 0.0;
    for (row, natural_height) in rows.iter().zip(row_natural_heights.iter()) {
        let row_height = natural_height * vertical_scale;
        let row_aspect_sum: f64 = row.iter().map(|&index| aspect_ratios[index]).sum();
        let mut x = 0.0;
        for &index in row {
            let width = viewport_width * (aspect_ratios[index] / row_aspect_sum);
            placements[index] = TileRect {
                x,
                y,
                width,
                height: row_height,
            };
            x += width;
        }
        y += row_height;
    }
    placements
}

/// Splits `aspect_ratios`' indices, in input order, into at most
/// `row_count` contiguous groups whose aspect-ratio sums are as close to
/// equal as a single forward greedy pass can make them: walk the cameras
/// in order, closing the current row once it has reached its fair share
/// (`total / row_count`) of the total aspect ratio. The final row absorbs
/// whatever is left, so every input camera lands in exactly one row and no
/// row is ever empty.
fn partition_into_rows(aspect_ratios: &[f64], row_count: usize) -> Vec<Vec<usize>> {
    let row_count = row_count.clamp(1, aspect_ratios.len());
    let target_per_row = aspect_ratios.iter().sum::<f64>() / count_as_f64(row_count);

    let mut rows = Vec::with_capacity(row_count);
    let mut current_row = Vec::new();
    let mut current_sum = 0.0;
    for (index, &aspect_ratio) in aspect_ratios.iter().enumerate() {
        let would_overflow_target = current_sum + aspect_ratio > target_per_row;
        // Never closes the `row_count`-th row itself -- it is always the
        // final, unconditional `rows.push` below, so it can absorb any
        // cameras a too-eager close would otherwise have nowhere to put.
        if !current_row.is_empty() && rows.len() + 1 < row_count && would_overflow_target {
            rows.push(std::mem::take(&mut current_row));
            current_sum = 0.0;
        }
        current_row.push(index);
        current_sum += aspect_ratio;
    }
    rows.push(current_row);
    rows
}

/// The height at which each row in `rows`, keeping every camera's own
/// aspect ratio, would together exactly fill `viewport_width`.
fn natural_row_heights(
    rows: &[Vec<usize>],
    aspect_ratios: &[f64],
    viewport_width: f64,
) -> Vec<f64> {
    rows.iter()
        .map(|row| {
            let aspect_sum: f64 = row.iter().map(|&index| aspect_ratios[index]).sum();
            viewport_width / aspect_sum
        })
        .collect()
}

/// Widens a small count (a row or camera count, always far below
/// `u32::MAX` in any real deployment) to `f64` for a ratio computation,
/// saturating rather than panicking on the reachable-only-in-theory
/// overflow case -- matching `monitor.rs`'s own `i32::try_from(index)`
/// stagger-delay idiom rather than an unchecked `as` cast.
fn count_as_f64(count: usize) -> f64 {
    u32::try_from(count).map_or(f64::MAX, f64::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANDSCAPE_16_9: (u32, u32) = (1920, 1080);
    const LANDSCAPE_4_3: (u32, u32) = (1280, 960);
    const PORTRAIT_9_16: (u32, u32) = (1080, 1920);

    const VIEWPORT_WIDTH: f64 = 1920.0;
    const VIEWPORT_HEIGHT: f64 = 1080.0;
    const EPSILON: f64 = 1e-6;

    fn assert_within_viewport_bounds(
        rects: &[TileRect],
        viewport_width: f64,
        viewport_height: f64,
    ) {
        for rect in rects {
            assert!(
                rect.x >= -EPSILON,
                "tile x {} is left of the viewport",
                rect.x
            );
            assert!(
                rect.y >= -EPSILON,
                "tile y {} is above the viewport",
                rect.y
            );
            assert!(
                rect.x + rect.width <= viewport_width + EPSILON,
                "tile right edge {} exceeds viewport width {viewport_width}",
                rect.x + rect.width,
            );
            assert!(
                rect.y + rect.height <= viewport_height + EPSILON,
                "tile bottom edge {} exceeds viewport height {viewport_height}",
                rect.y + rect.height,
            );
        }
    }

    fn rects_overlap(a: &TileRect, b: &TileRect) -> bool {
        a.x + EPSILON < b.x + b.width
            && b.x + EPSILON < a.x + a.width
            && a.y + EPSILON < b.y + b.height
            && b.y + EPSILON < a.y + a.height
    }

    fn assert_no_overlaps(rects: &[TileRect]) {
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!rects_overlap(a, b), "tiles overlap: {a:?} and {b:?}");
            }
        }
    }

    fn total_area(rects: &[TileRect]) -> f64 {
        rects.iter().map(|rect| rect.width * rect.height).sum()
    }

    #[test]
    fn one_camera_fills_the_entire_viewport() {
        let rects = pack_tiles(&[LANDSCAPE_16_9], VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_eq!(rects.len(), 1);
        assert!(rects[0].x.abs() < EPSILON);
        assert!(rects[0].y.abs() < EPSILON);
        assert!((rects[0].width - VIEWPORT_WIDTH).abs() < EPSILON);
        assert!((rects[0].height - VIEWPORT_HEIGHT).abs() < EPSILON);
    }

    #[test]
    fn two_mixed_aspect_cameras_stay_in_bounds_and_do_not_overlap() {
        let rects = pack_tiles(
            &[LANDSCAPE_16_9, PORTRAIT_9_16],
            VIEWPORT_WIDTH,
            VIEWPORT_HEIGHT,
        );
        assert_eq!(rects.len(), 2);
        assert_within_viewport_bounds(&rects, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_no_overlaps(&rects);
    }

    #[test]
    fn four_mixed_aspect_cameras_stay_in_bounds_and_do_not_overlap() {
        let cameras = [LANDSCAPE_16_9, LANDSCAPE_4_3, PORTRAIT_9_16, LANDSCAPE_16_9];
        let rects = pack_tiles(&cameras, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_eq!(rects.len(), 4);
        assert_within_viewport_bounds(&rects, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_no_overlaps(&rects);
    }

    #[test]
    fn five_mixed_aspect_cameras_stay_in_bounds_and_do_not_overlap() {
        let cameras = [
            LANDSCAPE_16_9,
            LANDSCAPE_4_3,
            PORTRAIT_9_16,
            LANDSCAPE_16_9,
            LANDSCAPE_4_3,
        ];
        let rects = pack_tiles(&cameras, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_eq!(rects.len(), 5);
        assert_within_viewport_bounds(&rects, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        assert_no_overlaps(&rects);
    }

    #[test]
    fn tiles_exactly_cover_the_viewport_with_no_gaps() {
        let cameras = [
            LANDSCAPE_16_9,
            LANDSCAPE_4_3,
            PORTRAIT_9_16,
            LANDSCAPE_16_9,
            LANDSCAPE_4_3,
        ];
        let rects = pack_tiles(&cameras, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
        let covered = total_area(&rects);
        let viewport_area = VIEWPORT_WIDTH * VIEWPORT_HEIGHT;
        assert!(
            (covered - viewport_area).abs() < viewport_area * 1e-6,
            "tiles cover {covered}, viewport is {viewport_area}",
        );
    }

    // Bounds and no-overlap alone would not catch a packer that gives every
    // tile an equal share regardless of aspect ratio -- the exact defect
    // D-2 rejected the `ceil(sqrt(n))` grid for. This pins the specific,
    // objectively checkable claim D-2 asked for: a wider-aspect camera
    // sharing a row gets proportionally more width, by the exact ratio the
    // module doc above derives, not just "some" or "a little" more.
    #[test]
    fn a_wider_camera_sharing_a_row_gets_proportionally_more_width() {
        // A very wide, short viewport leaves room for only one row
        // regardless of the row-count search, so both cameras are
        // guaranteed to land on it and are directly comparable at equal
        // height.
        let wide = (1920, 1080); // 16:9
        let narrow = (1280, 1080); // ~1.185:1 -- landscape, but clearly
        // narrower than `wide`, avoiding a tie.
        let rects = pack_tiles(&[wide, narrow], 3200.0, 100.0);
        assert_eq!(rects.len(), 2);
        assert!(
            (rects[0].y - rects[1].y).abs() < EPSILON,
            "both tiles should share the wall's single row"
        );
        assert!((rects[0].height - rects[1].height).abs() < EPSILON);

        let wide_ratio = f64::from(wide.0) / f64::from(wide.1);
        let narrow_ratio = f64::from(narrow.0) / f64::from(narrow.1);
        let expected_wide_width = 3200.0 * wide_ratio / (wide_ratio + narrow_ratio);
        let expected_narrow_width = 3200.0 * narrow_ratio / (wide_ratio + narrow_ratio);
        assert!((rects[0].width - expected_wide_width).abs() < EPSILON);
        assert!((rects[1].width - expected_narrow_width).abs() < EPSILON);
        assert!(
            rects[0].width > rects[1].width,
            "the wider-aspect camera should get more width"
        );
    }

    #[test]
    fn empty_camera_list_returns_no_tiles() {
        assert_eq!(pack_tiles(&[], VIEWPORT_WIDTH, VIEWPORT_HEIGHT), Vec::new());
    }

    #[test]
    fn non_positive_viewport_returns_no_tiles() {
        assert_eq!(
            pack_tiles(&[LANDSCAPE_16_9], 0.0, VIEWPORT_HEIGHT),
            Vec::new()
        );
        assert_eq!(
            pack_tiles(&[LANDSCAPE_16_9], VIEWPORT_WIDTH, -1.0),
            Vec::new()
        );
    }
}
