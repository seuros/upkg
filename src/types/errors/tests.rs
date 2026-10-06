use super::*;

#[test]
fn unsupported_bottle_display_includes_name() {
    let err = Error::UnsupportedBottle {
        name: "libheif".to_string(),
    };

    assert!(err.to_string().contains("libheif"));
}
