//! Template -> Scene behaviour, including every rejection the validator promises.

use std::fs;
use std::path::Path;

use nova_core::{ErrorCode, EvalKind, Limits, NodeKind, Prop};
use nova_template::{Compiled, load_value};
use serde_json::{Value, json};

fn load(dir: &Path, template: Value) -> Result<Compiled, Vec<nova_core::Error>> {
    load_value(&template, None, dir, &Limits::default())
}

fn base(layers: Value) -> Value {
    json!({
        "schema_version": "1.0",
        "composition": { "id": "t", "width": 1920, "height": 1080, "fps": 30, "duration_frames": 300 },
        "layers": layers
    })
}

fn rect(id: &str) -> Value {
    json!({ "id": id, "type": "rect", "size": { "width": 100, "height": 50 }, "fill": "#ff0000" })
}

/// Compile and expect exactly these (code, path-substring) failures to be among the errors.
fn expect_errors(template: Value, expected: &[(ErrorCode, &str)]) {
    let dir = tempfile::tempdir().unwrap();
    let errs = load(dir.path(), template).expect_err("template should be rejected");
    for (code, needle) in expected {
        assert!(
            errs.iter()
                .any(|e| e.code == *code && e.to_string().contains(needle)),
            "expected a {code:?} error mentioning `{needle}`, got:\n{}",
            errs.iter()
                .map(|e| format!("  {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn the_example_from_the_plan_compiles() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("assets")).unwrap();
    fs::write(
        dir.path().join("assets/product.png"),
        b"not decoded at compile time",
    )
    .unwrap();
    let template = json!({
      "schema_version": "1.0",
      "composition": {
        "id": "product-ad-001", "name": "Product Advertisement",
        "width": 1920, "height": 1080, "fps": { "num": 30, "den": 1 },
        "duration_frames": 450, "background": "#101522"
      },
      "assets": [{ "id": "product-photo", "type": "image", "src": "assets/product.png" }],
      "layers": [
        {
          "id": "headline", "type": "text", "start_frame": 0, "end_frame": 450,
          "transform": {
            "position": { "x": 180, "y": 250 }, "scale": { "x": 1, "y": 1 },
            "rotation": 0, "opacity": 1, "anchor": { "x": 0, "y": 0 }
          },
          "text": {
            "value": "CREATE SOMETHING BOLD", "font_family": "Inter", "font_size": 92,
            "font_weight": 700, "color": "#FFFFFF", "line_height": 1.05, "align": "left", "max_width": 1500
          },
          "animations": [
            { "property": "transform.position.y", "keyframes": [
                { "frame": 0, "value": 340, "easing": "ease_out_cubic" },
                { "frame": 18, "value": 250, "easing": "linear" } ] },
            { "property": "opacity", "keyframes": [
                { "frame": 0, "value": 0, "easing": "linear" },
                { "frame": 12, "value": 1, "easing": "linear" } ] }
          ]
        },
        {
          "id": "product-image", "type": "image", "asset": "product-photo",
          "start_frame": 0, "end_frame": 450, "fit": "contain",
          "transform": {
            "position": { "x": 1050, "y": 180 }, "scale": { "x": 1, "y": 1 },
            "rotation": 0, "opacity": 1, "anchor": { "x": 0.5, "y": 0.5 }
          }
        }
      ]
    });
    let c = load(dir.path(), template).unwrap_or_else(|e| panic!("{e:?}"));
    let s = &c.scene;
    assert_eq!((s.width, s.height, s.duration_frames), (1920, 1080, 450));
    assert_eq!(s.background.to_rgba8(), [0x10, 0x15, 0x22, 255]);
    assert_eq!(s.roots.len(), 2);
    // "Inter" is not bundled -> reproducibility warning, not an error.
    assert!(c.warnings.iter().any(|w| w.message.contains("Inter")));

    // Frame 0: headline is fully transparent, so only the image is drawn.
    assert_eq!(s.evaluate(0).nodes.len(), 1);
    let plan = s.evaluate(9);
    assert_eq!(plan.nodes.len(), 2);
    let headline = &plan.nodes[0];
    assert!(
        (headline.position[1] - 261.25).abs() < 1e-3,
        "ease_out_cubic halfway: {}",
        headline.position[1]
    );
    assert!((headline.opacity - 0.75).abs() < 1e-6);
    assert!(matches!(headline.kind, EvalKind::Text { size, .. } if size == 92.0));
    // After both animations finish the frames are identical.
    assert_eq!(s.evaluate(18), s.evaluate(449));
}

#[test]
fn groups_shift_their_childrens_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = rect("child");
    child["start_frame"] = json!(10);
    child["end_frame"] = json!(40);
    child["animations"] = json!([{ "property": "transform.position.x",
        "keyframes": [{ "frame": 0, "value": 0 }, { "frame": 10, "value": 100 }] }]);
    let group = json!({ "id": "scene2", "type": "group", "start_frame": 100, "end_frame": 130, "layers": [child] });
    let s = load(dir.path(), base(json!([group]))).unwrap().scene;

    let child = s.nodes.iter().find(|n| n.id == "child").unwrap();
    assert_eq!(
        child.origin, 110,
        "child local 0 = group start + child start"
    );
    assert_eq!(
        (child.start, child.end),
        (110, 130),
        "clipped to the group's end"
    );
    assert!(s.evaluate(109).nodes.is_empty());
    let plan = s.evaluate(115);
    let EvalKind::Group { children } = &plan.nodes[0].kind else {
        panic!("group expected")
    };
    assert_eq!(children[0].position[0], 50.0);
    assert!(s.evaluate(130).nodes.is_empty());
}

#[test]
fn uniform_scale_animation_drives_both_axes() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = rect("r");
    r["animations"] = json!([{ "property": "transform.scale",
        "keyframes": [{ "frame": 0, "value": 0.5, "easing": "spring" }, { "frame": 30, "value": 1 }] }]);
    let s = load(dir.path(), base(json!([r]))).unwrap().scene;
    let n = &s.nodes[0];
    assert!(!n.scale_x.is_const() && n.scale_x == n.scale_y);
    assert!(
        n.scale_x.max_abs() > 1.0,
        "spring overshoot is accounted for"
    );
}

#[test]
fn colour_and_shape_properties_animate() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = rect("r");
    r["stroke"] = json!({ "color": "#fff", "width": 4 });
    r["animations"] = json!([
        { "property": "fill.color", "keyframes": [{ "frame": 0, "value": "#000000" }, { "frame": 10, "value": "#ffffff" }] },
        { "property": "stroke.trim_end", "keyframes": [{ "frame": 0, "value": 0 }, { "frame": 10, "value": 1 }] },
        { "property": "size.width", "keyframes": [{ "frame": 0, "value": 10 }, { "frame": 10, "value": 110 }] },
        { "property": "corner_radius", "keyframes": [{ "frame": 0, "value": 0 }, { "frame": 10, "value": 20 }] }
    ]);
    let s = load(dir.path(), base(json!([r]))).unwrap().scene;
    let plan = s.evaluate(5);
    let EvalKind::Rect {
        width,
        radius,
        fill,
        stroke,
        ..
    } = &plan.nodes[0].kind
    else {
        panic!()
    };
    assert_eq!(*width, 60.0);
    assert_eq!(*radius, 10.0);
    assert_eq!(stroke.unwrap().trim_end, 0.5);
    let nova_core::EvalFill::Solid(c) = fill.unwrap() else {
        panic!()
    };
    assert_eq!(c.to_rgba8(), [128, 128, 128, 255]);
}

#[test]
fn hidden_and_never_visible_layers_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut hidden = rect("hidden");
    hidden["visible"] = json!(false);
    let mut late = rect("late");
    late["start_frame"] = json!(5000);
    let c = load(dir.path(), base(json!([hidden, late, rect("ok")]))).unwrap();
    assert_eq!(c.scene.roots.len(), 1);
    assert!(
        c.warnings
            .iter()
            .any(|w| w.message.contains("`late` is never visible"))
    );
}

#[test]
fn data_slots_are_bound_before_validation() {
    let dir = tempfile::tempdir().unwrap();
    let template = base(json!([{
        "id": "t", "type": "text",
        "text": { "value": "Hello {{name}}", "font_family": "X", "font_size": "{{size}}" }
    }]));
    let c = load_value(
        &template,
        Some(&json!({ "name": "Aina", "size": 64 })),
        dir.path(),
        &Limits::default(),
    )
    .unwrap();
    let NodeKind::Text(t) = &c.scene.nodes[0].kind else {
        panic!()
    };
    assert_eq!(t.value, "Hello Aina");
    assert_eq!(t.size, Prop::Const(64.0));

    let errs = load(dir.path(), template).unwrap_err();
    assert!(errs.iter().all(|e| e.code == ErrorCode::MissingData));
    assert_eq!(errs.len(), 2, "both missing slots are reported");
}

#[test]
fn rejects_unsupported_version() {
    let mut t = base(json!([rect("r")]));
    t["schema_version"] = json!("2.0");
    expect_errors(t, &[(ErrorCode::UnsupportedVersion, "2.0")]);
}

#[test]
fn rejects_unknown_fields_with_their_path() {
    let mut r = rect("r");
    r["colour"] = json!("#fff");
    expect_errors(
        base(json!([r])),
        &[(ErrorCode::InvalidTemplate, "layers[0]")],
    );
    let mut t = base(json!([rect("r")]));
    t["composition"]["script"] = json!("alert(1)");
    expect_errors(t, &[(ErrorCode::InvalidTemplate, "script")]);
}

#[test]
fn rejects_bad_composition() {
    let mut t = base(json!([rect("r")]));
    t["composition"]["width"] = json!(1921);
    expect_errors(t, &[(ErrorCode::Validation, "must be even")]);

    let mut t = base(json!([rect("r")]));
    t["composition"]["width"] = json!(20000);
    t["composition"]["duration_frames"] = json!(0);
    expect_errors(
        t,
        &[
            (ErrorCode::Limit, "exceeds the maximum"),
            (ErrorCode::Validation, "duration_frames"),
        ],
    );

    let mut t = base(json!([rect("r")]));
    t["composition"]["fps"] = json!({ "num": 30, "den": 0 });
    expect_errors(t, &[(ErrorCode::Validation, "composition.fps")]);
}

#[test]
fn rejects_duplicate_ids_even_across_nesting() {
    let group = json!({ "id": "g", "type": "group", "layers": [rect("dup")] });
    expect_errors(
        base(json!([rect("dup"), group])),
        &[(ErrorCode::Validation, "duplicate layer id `dup`")],
    );
}

#[test]
fn rejects_fields_of_the_wrong_layer_type() {
    let mut r = rect("r");
    r["text"] = json!({ "value": "x", "font_family": "A", "font_size": 10 });
    r["asset"] = json!("nope");
    expect_errors(
        base(json!([r])),
        &[
            (
                ErrorCode::Validation,
                "`text` is not valid on a `rect` layer",
            ),
            (ErrorCode::Validation, "`asset` is not valid"),
        ],
    );
}

#[test]
fn rejects_missing_required_content() {
    expect_errors(
        base(json!([
            { "id": "a", "type": "rect", "fill": "#fff" },
            { "id": "b", "type": "text" },
            { "id": "c", "type": "path", "fill": "#fff" },
            { "id": "d", "type": "solid" },
            { "id": "e", "type": "ellipse", "size": { "width": 5, "height": 5 } },
            { "id": "f", "type": "group" },
            { "id": "g", "type": "image" }
        ])),
        &[
            (ErrorCode::Validation, "a `rect` layer needs `size`"),
            (ErrorCode::Validation, "a `text` layer needs `text`"),
            (ErrorCode::Validation, "needs `d`"),
            (ErrorCode::Validation, "a `solid` layer needs `color`"),
            (ErrorCode::Validation, "needs `fill` and/or `stroke`"),
            (ErrorCode::Validation, "a `group` layer needs `layers`"),
            (ErrorCode::Validation, "`image` layer needs `asset`"),
        ],
    );
}

#[test]
fn rejects_bad_references_and_asset_kinds() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("logo.svg"), b"<svg/>").unwrap();
    let mut t = base(json!([
        { "id": "a", "type": "image", "asset": "ghost" },
        { "id": "b", "type": "image", "asset": "logo" }
    ]));
    t["assets"] = json!([
        { "id": "logo", "type": "svg", "src": "logo.svg" },
        { "id": "missing", "type": "image", "src": "nope.png" },
        { "id": "evil", "type": "image", "src": "../../etc/passwd" }
    ]);
    let errs = load(dir.path(), t).unwrap_err();
    let has = |code: ErrorCode, needle: &str| {
        errs.iter()
            .any(|e| e.code == code && e.to_string().contains(needle))
    };
    assert!(
        has(ErrorCode::Validation, "no asset with id `ghost`"),
        "{errs:?}"
    );
    assert!(
        has(ErrorCode::Validation, "asset `logo` is a svg"),
        "{errs:?}"
    );
    assert!(has(ErrorCode::Asset, "assets[1].src"), "{errs:?}");
    assert!(has(ErrorCode::Asset, "`..` is not allowed"), "{errs:?}");
}

#[test]
fn rejects_bad_animations() {
    let anim = |property: &str, keyframes: Value| {
        let mut r = rect("r");
        r["animations"] = json!([{ "property": property, "keyframes": keyframes }]);
        base(json!([r]))
    };
    expect_errors(
        anim("transform.wobble", json!([{ "frame": 0, "value": 1 }])),
        &[(ErrorCode::Validation, "cannot animate `transform.wobble`")],
    );
    expect_errors(
        anim("text.font_size", json!([{ "frame": 0, "value": 1 }])),
        &[(ErrorCode::Validation, "Animatable here")],
    );
    expect_errors(
        anim("opacity", json!([])),
        &[(ErrorCode::Validation, "at least one keyframe")],
    );
    expect_errors(
        anim(
            "opacity",
            json!([{ "frame": 10, "value": 0 }, { "frame": 10, "value": 1 }]),
        ),
        &[(ErrorCode::Validation, "strictly increase")],
    );
    expect_errors(
        anim("opacity", json!([{ "frame": 0, "value": "#fff" }])),
        &[(ErrorCode::Validation, "numeric")],
    );
    expect_errors(
        anim("fill.color", json!([{ "frame": 0, "value": 3 }])),
        &[(ErrorCode::Validation, "colour")],
    );
    expect_errors(
        anim(
            "opacity",
            json!([{ "frame": 0, "value": 0, "easing": "bouncy" }, { "frame": 5, "value": 1 }]),
        ),
        &[(ErrorCode::Validation, "unknown easing `bouncy`")],
    );
    expect_errors(
        anim(
            "opacity",
            json!([{ "frame": 0, "value": 0, "easing": { "cubic_bezier": [1.5, 0, 0, 1] } }, { "frame": 5, "value": 1 }]),
        ),
        &[(ErrorCode::Validation, "x1 and x2")],
    );
    expect_errors(
        anim("shadow.blur", json!([{ "frame": 0, "value": 1 }])),
        &[(ErrorCode::Validation, "no `shadow`")],
    );

    let mut r = rect("r");
    r["animations"] = json!([
        { "property": "opacity", "keyframes": [{ "frame": 0, "value": 1 }] },
        { "property": "transform.opacity", "keyframes": [{ "frame": 0, "value": 1 }] }
    ]);
    expect_errors(
        base(json!([r])),
        &[(ErrorCode::Validation, "animated more than once")],
    );
}

#[test]
fn rejects_bad_values() {
    let mut r = rect("r");
    r["opacity"] = json!(1.5);
    r["fill"] = json!("#12");
    r["end_frame"] = json!(0);
    r["blur"] = json!(-1);
    expect_errors(
        base(json!([r])),
        &[
            (ErrorCode::Validation, "between 0 and 1"),
            (ErrorCode::Validation, "invalid hex colour"),
            (ErrorCode::Validation, "must be greater than start_frame"),
            (ErrorCode::Validation, "blur"),
        ],
    );
    let grad = json!({ "id": "g", "type": "rect", "size": { "width": 1, "height": 1 },
        "fill": { "type": "linear", "stops": [{ "offset": 0.8, "color": "#000" }, { "offset": 0.2, "color": "#fff" }] } });
    expect_errors(
        base(json!([grad])),
        &[(ErrorCode::Validation, "must not decrease")],
    );
}

#[test]
fn enforces_limits() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        max_layers: 3,
        max_nesting_depth: 2,
        ..Limits::default()
    };
    let many = base(json!([rect("a"), rect("b"), rect("c"), rect("d")]));
    let errs = load_value(&many, None, dir.path(), &limits).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.code == ErrorCode::Limit && e.message.contains("more than 3 layers"))
    );

    let deep = base(json!([{ "id": "g1", "type": "group", "layers": [
        { "id": "g2", "type": "group", "layers": [ { "id": "g3", "type": "group", "layers": [rect("leaf")] } ] } ] }]));
    let errs = load_value(&deep, None, dir.path(), &limits).unwrap_err();
    assert!(
        errs.iter()
            .any(|e| e.code == ErrorCode::Limit && e.message.contains("nested deeper"))
    );
}

#[test]
fn schema_is_generated_and_describes_the_format() {
    let schema = nova_template::json_schema();
    let text = schema.to_string();
    for needle in [
        "schema_version",
        "duration_frames",
        "cubic_bezier",
        "trim_end",
        "font_family",
    ] {
        assert!(text.contains(needle), "schema should mention {needle}");
    }
}

fn caption(extra: Value) -> Value {
    let mut text = json!({ "value": "Jam émas hilang", "font_family": "X", "font_size": 40 });
    for (k, v) in extra.as_object().unwrap() {
        text[k] = v.clone();
    }
    base(json!([{ "id": "cap", "type": "text", "text": text }]))
}

#[test]
fn captions_compile_spans_reveal_and_background() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = caption(json!({
        "hug": true,
        "max_width": 600,
        "spans": [{ "start": 4, "end": 8, "color": "#f2c14e" }],
        "reveal": { "unit": "word", "count": 0, "hidden_color": "rgba(255,255,255,0.3)" },
        "background": { "color": "rgba(8,10,16,0.72)", "radius": 24, "padding": { "x": 34, "y": 20 },
                        "border_color": "#ffffff", "border_width": 2 }
    }));
    t["layers"][0]["animations"] = json!([{ "property": "text.reveal",
        "keyframes": [{ "frame": 0, "value": 1, "easing": "step" }, { "frame": 6, "value": 2, "easing": "step" }, { "frame": 12, "value": 3 }] }]);
    let s = load(dir.path(), t)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .scene;
    let NodeKind::Text(text) = &s.nodes[0].kind else {
        panic!()
    };
    // character indices 4..8 ("émas") become byte offsets 4..9
    assert_eq!((text.spans[0].start, text.spans[0].end), (4, 9));
    assert!(text.hug);
    assert_eq!(text.background.unwrap().padding, [34.0, 20.0, 34.0, 20.0]);

    let reveal_at = |f: u32| match &s.evaluate(f).nodes[0].kind {
        EvalKind::Text { reveal, .. } => reveal.unwrap(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        [
            reveal_at(0),
            reveal_at(5),
            reveal_at(6),
            reveal_at(11),
            reveal_at(12)
        ],
        [1, 1, 2, 2, 3]
    );
    // between steps the frames are identical, so they are not re-rendered
    assert_eq!(s.evaluate(1), s.evaluate(5));
    assert_ne!(s.evaluate(5), s.evaluate(6));
}

#[test]
fn rejects_bad_caption_options() {
    expect_errors(
        caption(json!({ "spans": [{ "start": 5, "end": 99, "color": "#fff" }] })),
        &[(ErrorCode::Validation, "end at most 15")],
    );
    expect_errors(
        caption(
            json!({ "spans": [{ "start": 4, "end": 8, "color": "#fff" }, { "start": 6, "end": 10, "color": "#fff" }] }),
        ),
        &[(ErrorCode::Validation, "must not overlap")],
    );
    expect_errors(
        caption(json!({ "background": { "radius": 4 } })),
        &[(ErrorCode::Validation, "needs `color` and/or `border_color`")],
    );
    expect_errors(
        caption(json!({ "background": { "color": "#000", "padding": { "x": 1 } } })),
        &[(ErrorCode::InvalidTemplate, "padding")],
    );
    let mut t = caption(json!({}));
    t["layers"][0]["animations"] =
        json!([{ "property": "text.reveal", "keyframes": [{ "frame": 0, "value": 1 }] }]);
    expect_errors(t, &[(ErrorCode::Validation, "no `reveal`")]);
}
