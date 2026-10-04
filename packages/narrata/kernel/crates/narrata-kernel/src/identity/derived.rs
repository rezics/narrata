/// Defines a fixed-width identity; the namespace literal includes its trailing colon.
#[macro_export]
macro_rules! derived_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                $crate::identity::text::format($prefix, &self.0, formatter)
            }
        }

        impl ::std::str::FromStr for $name {
            type Err = $crate::identity::IdParseError;

            fn from_str(input: &str) -> ::core::result::Result<Self, Self::Err> {
                $crate::identity::text::parse(input, $prefix).map(Self)
            }
        }
    };
}
