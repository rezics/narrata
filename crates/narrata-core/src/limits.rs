#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodeLimits {
    pub max_envelope_bytes: u64,
    pub max_payload_bytes: u64,
    pub max_value_depth: u32,
    pub max_string_bytes: u64,
    pub max_collection_items: u64,
    pub max_total_value_nodes: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        Self {
            max_envelope_bytes: 16 * 1024 * 1024,
            max_payload_bytes: 16 * 1024 * 1024,
            max_value_depth: 128,
            max_string_bytes: 1024 * 1024,
            max_collection_items: 1_000_000,
            max_total_value_nodes: 2_000_000,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProgramLimits {
    pub max_flows: u64,
    pub max_instructions: u64,
    pub max_constants: u64,
    pub max_globals: u64,
    pub max_locals_per_flow: u64,
    pub max_choices_per_instruction: u64,
    pub max_capabilities: u64,
    pub max_states: u64,
    pub max_regions: u64,
    pub max_transitions: u64,
    pub max_statechart_actions: u64,
    pub max_guard_depth: u32,
}

impl Default for ProgramLimits {
    fn default() -> Self {
        Self {
            max_flows: 16_384,
            max_instructions: 1_000_000,
            max_constants: 1_000_000,
            max_globals: 65_536,
            max_locals_per_flow: 65_536,
            max_choices_per_instruction: 4_096,
            max_capabilities: 1_024,
            max_states: 65_536,
            max_regions: 65_536,
            max_transitions: 262_144,
            max_statechart_actions: 1_000_000,
            max_guard_depth: 64,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLimits {
    pub max_call_depth: u64,
    pub max_stack_depth: u64,
    pub max_total_live_values: u64,
    pub max_active_states: u64,
    pub max_history_entries: u64,
    pub max_internal_events: u64,
    pub max_deferred_work: u64,
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_call_depth: 1_024,
            max_stack_depth: 16_384,
            max_total_live_values: 2_000_000,
            max_active_states: 65_536,
            max_history_entries: 65_536,
            max_internal_events: 65_536,
            max_deferred_work: 65_536,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MacrostepLimits {
    pub max_instructions: u64,
    pub max_calls: u64,
    pub max_logical_alloc_units: u64,
    pub max_microsteps: u64,
    pub max_internal_events: u64,
    pub runtime: RuntimeLimits,
}

impl Default for MacrostepLimits {
    fn default() -> Self {
        Self {
            max_instructions: 1_000_000,
            max_calls: 100_000,
            max_logical_alloc_units: 64 * 1024 * 1024,
            max_microsteps: 100_000,
            max_internal_events: 100_000,
            runtime: RuntimeLimits::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProgramLoadLimits {
    pub decode: DecodeLimits,
    pub program: ProgramLimits,
    pub runtime: RuntimeLimits,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SnapshotLoadLimits {
    pub decode: DecodeLimits,
    pub runtime: RuntimeLimits,
}
