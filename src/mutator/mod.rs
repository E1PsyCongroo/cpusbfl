mod elf_scheduled;
mod psbfl_mutator;
mod withw_mutator;
mod bounded_mutator;
mod selected_mutator;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MutationMetadata {
    pub(crate) mutated_pcs: HashSet<u64>,
}

libafl_bolts::impl_serdeany!(MutationMetadata);

pub(crate) use elf_scheduled::ELFHavocScheduledMutator;
pub(crate) use psbfl_mutator::{PSBFLMutationStrategy, PSBFLMutator};
pub(crate) use withw_mutator::WitHWMutator;
pub(crate) use bounded_mutator::BoundedMutator;
pub(crate) use selected_mutator::SelectedMutator;
