#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagnosticClass {
    Decode,
    Validation,
    Incompatible,
    LimitExceeded,
    InvalidInput,
    InvalidState,
    Conflict,
    BudgetSlice,
    MacrostepLimit,
    RuntimeFault,
    Store,
    Corrupt,
    Migration,
    Capability,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DiagnosticCode(pub &'static str);

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

pub mod codes {
    use super::DiagnosticCode;

    pub const DECODE_INVALID: DiagnosticCode = DiagnosticCode("NAR-D0001");
    pub const DECODE_NON_CANONICAL: DiagnosticCode = DiagnosticCode("NAR-D0002");
    pub const ENVELOPE_INVALID: DiagnosticCode = DiagnosticCode("NAR-D0003");
    pub const UNSUPPORTED_VERSION: DiagnosticCode = DiagnosticCode("NAR-I0001");
    pub const LIMIT_EXCEEDED: DiagnosticCode = DiagnosticCode("NAR-L0001");
    pub const DUPLICATE_ID: DiagnosticCode = DiagnosticCode("NAR-V0001");
    pub const MISSING_REFERENCE: DiagnosticCode = DiagnosticCode("NAR-V0002");
    pub const KIND_MISMATCH: DiagnosticCode = DiagnosticCode("NAR-V0003");
    pub const STACK_INVALID: DiagnosticCode = DiagnosticCode("NAR-V0004");
    pub const CONTROL_FLOW_INVALID: DiagnosticCode = DiagnosticCode("NAR-V0005");
    pub const INVALID_INPUT: DiagnosticCode = DiagnosticCode("NAR-N0001");
    pub const INVALID_STATE: DiagnosticCode = DiagnosticCode("NAR-N0002");
    pub const RUNTIME_ARITHMETIC: DiagnosticCode = DiagnosticCode("NAR-R0001");
    pub const RUNTIME_LIMIT: DiagnosticCode = DiagnosticCode("NAR-R0002");
    pub const RUNTIME_NO_CHOICES: DiagnosticCode = DiagnosticCode("NAR-R0003");
}
