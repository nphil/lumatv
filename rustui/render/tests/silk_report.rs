use lumatv_render::silk_report::parse;
use serde_json::json;

#[test]
fn report_defaults_are_scoped_and_invalid_fields_are_rejected() {
    let state = |silk| json!({"home": {"silk": silk}});
    let (id, silk) = parse(&state(json!({"id": 9}))).unwrap();
    assert_eq!(id.0, 9);
    assert_eq!((silk.amplitude, silk.depth, silk.scale, silk.detail), (4.0, 0.25, 1.0, 0.35));
    assert_eq!((silk.look.vignette, silk.look.specular, silk.look.bloom), (0.0, 0.0, 0.0));
    for bad in [json!({"id": 4294967296u64}), json!({"id": -1}), json!({"id": 1, "amplitude": 17}),
        json!({"id": 1, "depth": -0.1}), json!({"id": 1, "scale": 0.49}),
        json!({"id": 1, "detail": null}), json!({"id": 1, "vignette": 1.1}),
        json!({"id": 1, "specular": "0"}), json!({"id": 1, "bloom": -1})] {
        assert!(parse(&state(bad)).is_none());
    }
    assert!(parse(&json!({"home": {"silk": null}})).is_none());
    assert!(parse(&json!({})).is_none());
}
