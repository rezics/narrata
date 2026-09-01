#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ValueKindV0 {
    Null = 0,
    Bool = 1,
    I64 = 2,
    String = 3,
    List = 4,
    Record = 5,
    Variant = 6,
    Entity = 7,
}

impl ValueKindV0 {
    pub(crate) fn from_u64(value: u64) -> Option<Self> {
        match value {
            0 => Some(Self::Null),
            1 => Some(Self::Bool),
            2 => Some(Self::I64),
            3 => Some(Self::String),
            4 => Some(Self::List),
            5 => Some(Self::Record),
            6 => Some(Self::Variant),
            7 => Some(Self::Entity),
            _ => None,
        }
    }
}
