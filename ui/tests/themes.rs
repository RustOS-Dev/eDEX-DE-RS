//! Every shipped theme must parse with the full schema.

#[test]
fn shipped_themes_parse() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../themes");
    let themes = ui::theme::load_themes(&[dir.as_path()]);
    for name in [
        "tron",
        "matrix",
        "amber",
        "cyborg",
        "blade",
        "apollo",
        "interstellar",
        "horizon",
        "navy",
        "nord",
        "red",
        "purple",
    ] {
        let t = themes
            .get(name)
            .unwrap_or_else(|| panic!("theme {name} missing or invalid"));
        assert_eq!(t.name, name);
        assert!(t.palette.iter().all(|c| c[3] > 0.0));
    }
}
