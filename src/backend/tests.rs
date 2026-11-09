use super::*;
use test_support::assert_spec;

#[test]
fn command_spec_new_creates_spec() {
    let spec = CommandSpec::new("ls", vec!["-la".to_string()]);
    assert_spec(&spec, "ls", &["-la"]);
}
