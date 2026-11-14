//! Alignment in template coordinates; the magnetic tolerance is in UI points.
use crate::template::Template;

const SNAP_DISTANCE: f64 = 6.0;
const EXACT: f64 = 0.000001;

#[derive(Default, Debug)]
pub struct Guides {
    pub vertical: Vec<f64>,
    pub horizontal: Vec<f64>,
}

pub struct AlignedDrag {
    pub x: f64,
    pub y: f64,
    pub guides: Guides,
}

fn nearest_delta(anchors: [f64; 3], targets: &[f64], tolerance: f64) -> f64 {
    let mut best = None::<f64>;
    for target in targets {
        for anchor in anchors {
            let delta = target - anchor;
            if delta.abs() <= tolerance && best.is_none_or(|b| delta.abs() < b.abs()) {
                best = Some(delta);
            }
        }
    }
    best.unwrap_or(0.0)
}

fn matching_guides(anchors: [f64; 3], targets: &[f64]) -> Vec<f64> {
    let mut guides = Vec::<f64>::new();
    for target in targets {
        if anchors.iter().any(|a| (a - target).abs() < EXACT)
            && !guides.iter().any(|g| (g - target).abs() < EXACT)
        {
            guides.push(*target);
        }
    }
    guides
}

/// Align visual edges/centers with the canvas or another placement. The
/// selected box is excluded, and unconstrained pointer coordinates are kept
/// by the caller so moving away from a guide naturally releases the snap.
pub fn align_drag(
    template: &Template,
    selected: usize,
    requested: (f64, f64),
    scale: f64,
    enabled: bool,
) -> AlignedDrag {
    let mut result = AlignedDrag {
        x: requested.0,
        y: requested.1,
        guides: Guides::default(),
    };
    if !enabled {
        return result;
    }
    let slot = &template.slots[selected];
    let bounds = slot
        .bounds()
        .translated(requested.0 - slot.x, requested.1 - slot.y);
    let mut xs = vec![template.width as f64 * 0.5, 0.0, template.width as f64];
    let mut ys = vec![template.height as f64 * 0.5, 0.0, template.height as f64];
    for (i, other) in template.slots.iter().enumerate() {
        if i == selected {
            continue;
        }
        let b = other.bounds();
        xs.extend(b.x_anchors());
        ys.extend(b.y_anchors());
    }
    let dx = nearest_delta(bounds.x_anchors(), &xs, SNAP_DISTANCE / scale);
    let dy = nearest_delta(bounds.y_anchors(), &ys, SNAP_DISTANCE / scale);
    result.x += dx;
    result.y += dy;
    let aligned = bounds.translated(dx, dy);
    result.guides.vertical = matching_guides(aligned.x_anchors(), &xs);
    result.guides.horizontal = matching_guides(aligned.y_anchors(), &ys);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::PhotoSlot;

    fn scene() -> Template {
        let mut t = Template::builtin(false);
        t.width = 1000;
        t.height = 800;
        t.slots = vec![
            PhotoSlot::new(90.0, 100.0, 100.0, 80.0, 0),
            PhotoSlot::new(300.0, 400.0, 100.0, 80.0, 1),
        ];
        t
    }

    #[test]
    fn aligns_peer_edges_and_centers_and_canvas_center() {
        let t = scene();
        let a = align_drag(&t, 0, (297.0, 250.0), 1.0, true);
        assert_eq!((a.x, a.y), (300.0, 250.0));
        assert_eq!(a.guides.vertical, vec![300.0, 350.0, 400.0]);
        assert!(a.guides.horizontal.is_empty());
        let a = align_drag(&t, 0, (452.0, 362.0), 1.0, true);
        assert_eq!((a.x, a.y), (450.0, 360.0));
        assert!(a.guides.vertical.contains(&500.0));
        assert!(a.guides.horizontal.contains(&400.0));
    }

    #[test]
    fn tolerance_is_screen_based_and_free_movement_bypasses_it() {
        let t = scene();
        assert_eq!(align_drag(&t, 0, (290.0, 250.0), 1.0, true).x, 290.0);
        assert_eq!(align_drag(&t, 0, (290.0, 250.0), 0.5, true).x, 300.0);
        assert_eq!(align_drag(&t, 0, (297.0, 250.0), 2.0, true).x, 300.0);
        let free = align_drag(&t, 0, (297.0, 250.0), 1.0, false);
        assert_eq!((free.x, free.y), (297.0, 250.0));
        assert!(free.guides.vertical.is_empty());
        assert!(free.guides.horizontal.is_empty());
    }

    #[test]
    fn rotated_boxes_use_visible_bounds_and_do_not_snap_to_themselves() {
        let mut t = scene();
        t.slots[0].rotation = 90.0;
        let a = align_drag(&t, 0, (378.0, 250.0), 1.0, true);
        assert_eq!(a.x, 380.0); // rotated left edge is x - height = 300
        assert!(a.guides.vertical.contains(&300.0));
        t.slots.truncate(1);
        let a = align_drag(&t, 0, (92.0, 102.0), 1.0, true);
        assert_eq!((a.x, a.y), (92.0, 102.0));
        assert!(a.guides.vertical.is_empty());
    }
}
