use narrata_core::{ChoiceId, FlowId};

fn takes_flow(_: FlowId) {}

fn main() {
    let choice = ChoiceId::from_u128(1);
    takes_flow(choice);
}

