#![allow(clippy::panic)]

#[test]
fn identities_are_not_interchangeable() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/wrong_identity.rs");
}
