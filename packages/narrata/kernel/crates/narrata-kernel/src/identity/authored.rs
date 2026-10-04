/// Defines a fixed-width identity; the namespace literal includes its trailing colon.
#[macro_export]
macro_rules! authored_id {
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 16]);

        impl $name {
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            pub const fn from_u128(value: u128) -> Self {
                Self(value.to_be_bytes())
            }

            pub const fn as_bytes(&self) -> &[u8; 16] {
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
