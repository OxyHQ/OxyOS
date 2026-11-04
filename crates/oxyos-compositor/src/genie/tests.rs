use super::*;

/// The page's recorded minimize: an 800 × 500 window at (300, 140) on an 1800 × 1169 point display,
/// into a 35 × 55 slot at (990, 1114).
const WINDOW: Rect = Rect {
    x: 300.0,
    y: 140.0,
    width: 800.0,
    height: 500.0,
};
const SLOT: Rect = Rect {
    x: 990.0,
    y: 1114.0,
    width: 35.0,
    height: 55.0,
};

fn close(a: f64, b: f64, within: f64) -> bool {
    (a - b).abs() <= within
}

/// The rows of a bottom mesh: where each is on the screen (y), its left and right ends (x), and
/// where it is on the window (y).
fn rows(mesh: &Mesh) -> Vec<[f32; 4]> {
    assert_eq!(mesh.columns, 2);
    (0..mesh.rows)
        .map(|r| {
            let (left, right) = (mesh.points[2 * r], mesh.points[2 * r + 1]);
            assert_eq!((left[0], right[0]), (0.0, 800.0));
            assert_eq!((left[1], left[3]), (right[1], right[3]));
            [left[3], left[2], right[2], left[1]]
        })
        .collect()
}

#[test]
fn the_phases_last_and_meet_as_the_page_works_them_out() {
    // The bend's distance is the larger of the sides' moves summed over 8, (690 + 75) / 8 = 95.625,
    // and twice the overlap, none: trunc(ln(95.625 / 2 + 1)) = trunc(3.888) = 3, 150. The slide's
    // is the travel, 1114 − 140 = 974: trunc(ln(488)) = trunc(6.190) = 6, 300. The gap is
    // 974 − 500 = 474, which the slide's cosine covers at acos(1 − 2·474/974)/π = 0.4915 of it,
    // 147.45: the bend (150) would finish later, so the slide waits lead = 2.55, and the bend is
    // done at 150 / 302.55 = 0.4958 of the progress, as the page's figure has it ("0.50, as the
    // bottom edge reaches the slot").
    let reach = 300.0 * (1.0 - 2.0 * 474.0 / 974.0f64).acos() / PI;
    assert!(close(reach, 147.45, 0.01), "{reach}");
    let lead = 150.0 - reach;
    let at = |p: f64| schedule(p, WINDOW, SLOT, Edge::Bottom);
    assert_eq!(at(0.0).gap, 474.0);
    assert!(close(at(0.0).settled, 150.0 / (300.0 + lead), 1e-12));
    assert!(close(at(0.0).settled, 0.4958, 1e-4));
    // Each phase is half a cosine of its own time: at a quarter, along = 302.55 · 0.25 = 75.64,
    // bent = 0.5 − 0.5·cos(π·75.64/150) and slid = 0.5 − 0.5·cos(π·(75.64 − 2.55)/300).
    let ease = |t: f64| 0.5 - 0.5 * (PI * t).cos();
    for p in [0.1, 0.25, 0.4, 0.6, 0.75, 0.9] {
        let along = (300.0 + lead) * p as f32 as f64;
        let expected_bent = ease((along / 150.0).clamp(0.0, 1.0));
        let expected_poured = ease(((along - lead) / 300.0).clamp(0.0, 1.0));
        assert!(close(at(p).bent, expected_bent, 1e-12), "bent at {p}");
        assert!(close(at(p).poured, expected_poured, 1e-12), "poured at {p}");
    }
    assert!(close(at(0.25).bent, 0.506_674, 1e-6));
    assert!(close(at(0.25).poured, 0.139_439, 1e-6));
    // The bend is whole from 0.4958 on; the window slides to the end.
    assert_eq!(at(0.5).bent, 1.0);
    assert!(at(0.49).bent < 1.0);
    assert_eq!((at(0.0).bent, at(0.0).poured), (0.0, 0.0));
    assert_eq!((at(1.0).bent, at(1.0).poured), (1.0, 1.0));
}

#[test]
fn the_recorded_minimize_has_the_pages_rows_at_its_four_points() {
    // The page's figure: 5 rows at 0, 10 at a quarter, 16 at half (15 quads, the peak), 6 at three
    // quarters.
    let counts: Vec<usize> = [0.0, 0.25, 0.5, 0.75]
        .iter()
        .map(|&p| mesh(p, WINDOW, SLOT, Edge::Bottom).rows)
        .collect();
    assert_eq!(counts, [5, 10, 16, 6]);
}

#[test]
fn open_the_mesh_is_the_window_split_into_pieces_under_128_points() {
    // Nothing bent or slid: the sides are straight, and 500 points are halved to pieces of 125.
    let open = rows(&mesh(0.0, WINDOW, SLOT, Edge::Bottom));
    let expected: Vec<[f32; 4]> = (0..5)
        .map(|k| {
            let at = 125.0 * k as f32;
            [140.0 + at, 300.0, 1100.0, at]
        })
        .collect();
    assert_eq!(open, expected);
}

#[test]
fn a_quarter_in_the_sides_are_the_cosine_from_the_windows_corners_to_the_slots() {
    let quarter = rows(&mesh(0.25, WINDOW, SLOT, Edge::Bottom));
    // The window's top has slid 974 · 0.139439 = 135.81 points; the funnel starts where it was
    // (140), so the sides there are the cosine 135.81 points along a period of 974:
    // 0.5 − 0.5·cos(π·135.81/974) = 0.0477 of the sides' bend, 690 · 0.5067 and −75 · 0.5067.
    let slid = 974.0 * schedule(0.25, WINDOW, SLOT, Edge::Bottom).poured;
    let bent = schedule(0.25, WINDOW, SLOT, Edge::Bottom).bent;
    let side = |y: f64| 0.5 - 0.5 * ((y - 140.0) * PI / 974.0).cos();
    for row in &quarter {
        let y = f64::from(row[0]);
        assert!(close(
            f64::from(row[1]),
            300.0 + 690.0 * bent * side(y),
            1e-3
        ));
        assert!(close(
            f64::from(row[2]),
            1100.0 - 75.0 * bent * side(y),
            1e-3
        ));
    }
    assert!(close(f64::from(quarter[0][0]), 140.0 + slid, 1e-3));
    assert!(close(f64::from(quarter[9][0]), 640.0 + slid, 1e-3));
    // The pieces are halved until each is within 0.6 points of straight: shorter near the top,
    // where the cosine bends most (31.25 points), longer below (62.5 and 125).
    let gaps: Vec<f64> = quarter
        .windows(2)
        .map(|w| f64::from(w[1][3] - w[0][3]))
        .collect();
    let halved = [31.25, 31.25, 31.25, 31.25, 62.5, 62.5, 125.0, 62.5, 62.5];
    assert!(
        gaps.iter().zip(halved).all(|(&a, b)| close(a, b, 1e-4)),
        "{gaps:?}"
    );
    // The page's own code's values for it, to the float.
    assert_eq!(
        quarter[4].map(f64::from),
        [
            400.813_385_009_765_6,
            358.290_069_580_078_1,
            1_093.664_062_5,
            125.000_015_258_789_06
        ],
    );
}

#[test]
fn rows_past_the_slots_near_edge_are_clamped_to_it_and_a_last_row_is_added() {
    // At three quarters the window has slid 974 · 0.8512 = 829 points, past the gap of 474: its
    // bottom (1469) is clamped to the slot's top (1114), with the slot's width there, and a last
    // row at the slot's bottom (1169) holds the rest of the window.
    let late = rows(&mesh(0.75, WINDOW, SLOT, Edge::Bottom));
    assert_eq!(late.len(), 6);
    assert_eq!(late[4], [1114.0, 990.0, 1025.0, late[4][3]]);
    assert_eq!(late[5][..3], [1169.0, 990.0, 1025.0]);
    assert!(close(f64::from(late[5][3]), 500.0, 1e-4));
    // In the slot: the window's whole length between the first row and the slot's bottom.
    assert!(close(f64::from(late[4][3]), 362.462, 1e-3));
    // In it: the window's rows are in the slot's rectangle, its first two at its top.
    let end = rows(&mesh(1.0, WINDOW, SLOT, Edge::Bottom));
    assert_eq!(
        end,
        [
            [1114.0, 990.0, 1025.0, 0.0],
            [1114.0, 990.0, 1025.0, 0.0],
            [1169.0, 990.0, 1025.0, 500.0]
        ]
    );
    // Before the bottom reaches the slot, no row is clamped and none is added.
    let half = rows(&mesh(0.45, WINDOW, SLOT, Edge::Bottom));
    assert!(half.iter().all(|row| row[0] <= 1114.0));
    assert_eq!(half.last().unwrap()[3], 500.0);
}

#[test]
fn every_row_goes_down_the_screen_and_the_window_in_order() {
    for k in 0..=100 {
        let p = f64::from(k) / 100.0;
        let r = rows(&mesh(p, WINDOW, SLOT, Edge::Bottom));
        assert!(r.len() >= 3 && r.len() <= 17, "{} rows at {p}", r.len());
        for pair in r.windows(2) {
            assert!(pair[1][0] >= pair[0][0], "at {p}: {pair:?}");
            assert!(pair[1][3] >= pair[0][3], "at {p}: {pair:?}");
        }
        // The window's whole length, from its top to its bottom (its 32-bit rounding aside).
        assert_eq!(r[0][3], 0.0);
        assert!(close(f64::from(r.last().unwrap()[3]), 500.0, 1e-4));
    }
}

/// One minimize the page's own code worked out (`reference.json`): the progress, the edge, the
/// window and the slot, the phases and the mesh's vertices.
#[derive(serde::Deserialize)]
struct Reference {
    p: f64,
    edge: String,
    win: [f64; 4],
    slot: [f64; 4],
    bent: f64,
    poured: f64,
    columns: usize,
    rows: usize,
    points: Vec<f64>,
}

/// How many of `references`' vertices differ from the port's, and by at most how much.
fn compare(references: &[Reference]) -> (usize, usize, f64) {
    let (mut differing, mut total, mut worst) = (0, 0, 0.0f64);
    for case in references {
        let edge = match case.edge.as_str() {
            "bottom" => Edge::Bottom,
            "top" => Edge::Top,
            "left" => Edge::Left,
            _ => Edge::Right,
        };
        let rect = |r: [f64; 4]| Rect::new(r[0], r[1], r[2], r[3]);
        let (window, slot) = (rect(case.win), rect(case.slot));
        let phases = schedule(case.p, window, slot, edge);
        assert!(close(phases.bent, case.bent, 1e-12), "{}", case.edge);
        assert!(close(phases.poured, case.poured, 1e-12), "{}", case.edge);
        let ported = mesh(case.p, window, slot, edge);
        assert_eq!(
            (ported.columns, ported.rows),
            (case.columns, case.rows),
            "{} at {} of {:?} into {:?}",
            case.edge,
            case.p,
            case.win,
            case.slot
        );
        let flat: Vec<f64> = ported
            .points
            .iter()
            .flat_map(|v| v.iter().map(|&x| f64::from(x)))
            .collect();
        // Each reference is a 32-bit float written out in full: read back to the float it is.
        for (a, b) in flat.iter().zip(&case.points) {
            let b = f64::from(*b as f32);
            total += 1;
            if *a != b {
                differing += 1;
                worst = worst.max((a - b).abs());
            }
        }
    }
    (differing, total, worst)
}

#[test]
fn the_port_gives_the_pages_own_meshes_with_the_dock_on_every_side() {
    // 48 random minimizes on the page's 1800 × 1169 display, the Dock on each side in turn, some
    // windows overlapping it, each run through the page's own code (its `genieMesh`, from its
    // published bundle, under Node) for its phases and vertices. Every vertex is the same float;
    // only `cos`, `acos` and `ln` may round a last bit differently from V8's, which a 32-bit
    // rounding nearly always hides.
    let references: Vec<Reference> =
        serde_json::from_str(include_str!("reference.json")).expect("the reference meshes");
    assert_eq!(references.len(), 48);
    let (differing, total, worst) = compare(&references);
    assert!(total > 2000, "{total}");
    assert!(
        differing * 1000 <= total && worst < 1e-3,
        "{differing} of {total} differ, by up to {worst}"
    );
    // `OXYOS_GENIE_REFERENCE`: a larger set made the same way (the page checked its port on 7,200).
    if let Ok(path) = std::env::var("OXYOS_GENIE_REFERENCE") {
        let text = std::fs::read_to_string(path).expect("the reference file");
        let references: Vec<Reference> = serde_json::from_str(&text).expect("reference meshes");
        let (differing, total, worst) = compare(&references);
        eprintln!(
            "{} minimizes: {differing} of {total} vertices differ, by up to {worst}",
            references.len()
        );
        assert!(differing * 1000 <= total && worst < 1e-3);
    }
}
