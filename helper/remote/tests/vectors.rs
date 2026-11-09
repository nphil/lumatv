//! `vectors.json` is what the engine produces, so LumaRemoteKit's tests, which run the same file, test against the TV.

#[test]
fn the_checked_in_vectors_are_the_engines() {
    let file: serde_json::Value = serde_json::from_str(include_str!("vectors.json")).unwrap();
    assert!(file == lumaremote::vectors::generate(), "tests/vectors.json is out of date: cargo run -p lumaremote -- vectors remote/tests/vectors.json");
}

#[test]
fn refused_codes_are_refused() {
    for typed in lumaremote::vectors::REFUSED {
        assert_eq!(lumaremote::pairing::normalize(typed), None, "{typed}");
    }
}
