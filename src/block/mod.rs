pub mod dfb;
pub mod mgr;

pub use dfb::DataFlowBlock;
pub use mgr::BlockManager;

/// Block type classification
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CircuitType {
    COMB,
    SEQ,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlockType {
    ModuleInput,
    ModuleOutput,
    Always(CircuitType),
    Assign,
}

/// Trait for a parsed RTL block — only the metadata needed for SBFL
pub trait Block {
    fn bid(&self) -> u64;
    fn module_name(&self) -> &str;
    fn scope(&self) -> &str;
    fn block_type(&self) -> &BlockType;
    /// Original source file line numbers covered by this block
    fn line_ranges(&self) -> Vec<u32>;
}
