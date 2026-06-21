use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use sv_parser::{Define, NodeEvent, RefNode, SyntaxTree, parse_sv, unwrap_node};

use crate::block::{Block, DataFlowBlock, dfb};

pub struct BlockManager {
    blocks_by_scope: HashMap<String, (Vec<DataFlowBlock>, Arc<SyntaxTree>)>,
}

impl BlockManager {
    pub fn new<P: AsRef<Path>>(
        rtl_files: &[P],
        includes: &[PathBuf],
        top_module: &str,
        top_scope: &str,
    ) -> Self {
        let defines: HashMap<String, Option<Define>> = HashMap::new();

        let mut module_tree_map: HashMap<String, Arc<SyntaxTree>> = HashMap::new();
        let mut module_code_map: HashMap<String, String> = HashMap::new();

        for file in rtl_files {
            let file = file.as_ref();
            let path_str = file.to_str().unwrap_or("");
            match parse_sv(path_str, &defines, includes, false, false) {
                Ok((tree, _)) => {
                    let mname = get_module_name(&tree).unwrap_or_else(|| "unknown".to_string());
                    let code = std::fs::read_to_string(file).unwrap_or_default();
                    module_code_map.insert(mname.clone(), code);
                    module_tree_map.insert(mname, Arc::new(tree));
                }
                Err(e) => {
                    log::warn!("[parse failed] {}: {:?}", file.display(), e);
                }
            }
        }

        let top_tree = module_tree_map
            .get(top_module)
            .unwrap_or_else(|| {
                panic!(
                    "Top module '{}' not found. Available: {:?}",
                    top_module,
                    module_tree_map.keys()
                )
            })
            .clone();

        let mut blocks_by_scope: HashMap<String, (Vec<DataFlowBlock>, Arc<SyntaxTree>)> =
            HashMap::new();
        let mut queue: VecDeque<(String, Arc<SyntaxTree>)> = VecDeque::new();
        queue.push_back((top_scope.to_string(), top_tree));

        while let Some((cur_scope, cur_tree)) = queue.pop_front() {
            let mname = get_module_name(&cur_tree).unwrap_or_else(|| "unknown".to_string());
            let code = module_code_map.get(&mname).cloned().unwrap_or_default();
            let blocks = dfb::parse_module_blocks(&cur_tree, &cur_scope, &code);
            blocks_by_scope.insert(cur_scope.clone(), (blocks, cur_tree.clone()));

            // Find submodule instantiations
            let mut scope_prefix: Vec<String> = vec![];
            for event in cur_tree.clone().into_iter().event() {
                match event {
                    NodeEvent::Enter(RefNode::GenerateBlockIdentifier(node)) => {
                        if let Some(ident) = cur_tree.get_str(&node.nodes.0) {
                            scope_prefix.push(ident.trim().to_string());
                        }
                    }
                    NodeEvent::Leave(RefNode::GenerateBlock(_)) => {
                        scope_prefix.pop();
                    }
                    NodeEvent::Enter(RefNode::ModuleInstantiation(inst)) => {
                        let sub_module = get_module_name_from_instantiation(&cur_tree, &inst);
                        let dut_name = get_dut_name_from_instantiation(&cur_tree, &inst);
                        if let (Some(sub_module), Some(dut_name)) = (sub_module, dut_name) {
                            let prefix = scope_prefix.join(".");
                            let next_scope = if prefix.is_empty() {
                                format!("{}.{}", cur_scope, dut_name)
                            } else {
                                format!("{}.{}.{}", cur_scope, prefix, dut_name)
                            };
                            if let Some(next_tree) = module_tree_map.get(&sub_module) {
                                queue.push_back((next_scope, next_tree.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Self { blocks_by_scope }
    }

    pub fn get_all_blocks(&self) -> Vec<&DataFlowBlock> {
        self.blocks_by_scope
            .values()
            .flat_map(|(blocks, _)| blocks.iter())
            .collect()
    }

    pub fn dump_blocks_distribution(
        &self,
        output_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let data: Vec<_> = self
            .blocks_by_scope
            .iter()
            .flat_map(|(scope, (blocks, _))| {
                blocks.iter().map(move |block| {
                    serde_json::json!({
                        "bid": block.bid(),
                        "scope": scope,
                        "module": block.module_name(),
                        "type": format!("{:?}", block.block_type()),
                        "lines": block.line_ranges(),
                    })
                })
            })
            .collect();
        save_data_to_json(&data, format!("{}/blocks.json", output_path))?;
        Ok(())
    }
}

fn get_module_name(tree: &SyntaxTree) -> Option<String> {
    for node in tree {
        match node {
            RefNode::ModuleDeclarationNonansi(x) => {
                if let Some(RefNode::Identifier(identifier)) = unwrap_node!(x, Identifier) {
                    let identifier = identifier.clone();
                    if let Some(name) = tree.get_str(&identifier) {
                        return Some(name.trim().to_string());
                    }
                }
            }
            RefNode::ModuleDeclarationAnsi(x) => {
                if let Some(RefNode::Identifier(identifier)) = unwrap_node!(x, Identifier) {
                    let identifier = identifier.clone();
                    if let Some(name) = tree.get_str(&identifier) {
                        return Some(name.trim().to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn get_module_name_from_instantiation(
    tree: &SyntaxTree,
    inst: &sv_parser::ModuleInstantiation,
) -> Option<String> {
    if let Some(RefNode::ModuleIdentifier(identifier)) = unwrap_node!(inst, ModuleIdentifier) {
        tree.get_str(&identifier.nodes.0)
            .map(|s| s.trim().to_string())
    } else {
        None
    }
}

fn get_dut_name_from_instantiation(
    tree: &SyntaxTree,
    inst: &sv_parser::ModuleInstantiation,
) -> Option<String> {
    if let Some(RefNode::HierarchicalInstance(instance)) = unwrap_node!(inst, HierarchicalInstance)
    {
        if let Some(RefNode::NameOfInstance(identifier)) = unwrap_node!(instance, NameOfInstance) {
            tree.get_str(&identifier.nodes.0.nodes.0)
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    } else {
        None
    }
}

fn save_data_to_json<T: Serialize, P: AsRef<Path>>(
    data: &T,
    path: P,
) -> Result<(), Box<dyn std::error::Error>> {
    let json = serde_json::to_string_pretty(data)?;
    std::fs::write(path, json)?;
    Ok(())
}
