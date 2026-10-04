#![allow(clippy::unwrap_used)]

// Invoke the macros outside their defining crate, with no imports of their
// implementation dependencies. Names in the caller must not capture helpers.
mod domain {
    struct IdParseError;
    mod text {}
    type Result = ();
    narrata_kernel::authored_id!(NodeId, "node:");
    narrata_kernel::derived_id!(PackageId, "package:");
    // Exercise the shadow names so that all-targets linting checks this scope.
    const _: Result = ();
    const _: IdParseError = IdParseError;
}

use domain::{NodeId, PackageId};
use narrata_kernel::identity::{IdParseError, text::parse};

#[test]
fn domain_id_macros_preserve_width_namespace_and_lowercase_text() {
    let node = NodeId::from_u128(0xabcdef);
    assert_eq!(node.to_string(), "node:00000000000000000000000000abcdef");
    assert_eq!(*node.as_bytes(), 0xabcdef_u128.to_be_bytes());
    assert_eq!(node, NodeId::from_bytes(*node.as_bytes()));
    assert_eq!(
        node.to_string()
            .to_uppercase()
            .replacen("NODE:", "node:", 1)
            .parse::<NodeId>()
            .unwrap(),
        node
    );
    let package = PackageId::from_bytes([0xab; 32]);
    assert_eq!(package.to_string(), format!("package:{}", "ab".repeat(32)));
    assert_eq!(package.to_string().parse::<PackageId>().unwrap(), package);
    assert_eq!(package.as_bytes(), &[0xab; 32]);
    assert!(NodeId::from_u128(1) < NodeId::from_u128(2));
}

#[test]
fn identity_text_rejects_wrong_namespaces_widths_and_hex() {
    assert_eq!(
        "wrong:00".parse::<NodeId>(),
        Err(IdParseError::WrongNamespace { expected: "node:" })
    );
    for input in [
        "node:",
        "node:00",
        "node:0000000000000000000000000000000000",
    ] {
        assert_eq!(
            input.parse::<NodeId>(),
            Err(IdParseError::WrongLength { expected: 32 })
        );
    }
    assert_eq!(
        format!("node:{}", "z".repeat(32)).parse::<NodeId>(),
        Err(IdParseError::InvalidHex)
    );
    assert_eq!(
        "package:00".parse::<PackageId>(),
        Err(IdParseError::WrongLength { expected: 64 })
    );
    assert_eq!(parse::<2>("host:abcd", "host:").unwrap(), [0xab, 0xcd]);
    assert!(parse::<16>("node:00000000000000000000000000000001 ", "node:").is_err());
}
