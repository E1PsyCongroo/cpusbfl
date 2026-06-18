use crate::block::{Block, BlockType, CircuitType};
use sv_parser::{unwrap_node, AlwaysKeyword, PortDirection, RefNode, SyntaxTree};

pub struct DataFlowBlock {
    bid: u64,
    module_name: String,
    scope: String,
    block_type: BlockType,
    lines: Vec<u32>,
}

impl Block for DataFlowBlock {
    fn bid(&self) -> u64 { self.bid }
    fn module_name(&self) -> &str { &self.module_name }
    fn scope(&self) -> &str { &self.scope }
    fn block_type(&self) -> &BlockType { &self.block_type }
    fn line_ranges(&self) -> Vec<u32> { self.lines.clone() }
}

impl DataFlowBlock {
    pub fn new(bid: u64, module_name: &str, scope: &str, block_type: BlockType) -> Self {
        Self { bid, module_name: module_name.to_string(), scope: scope.to_string(), block_type, lines: Vec::new() }
    }
    fn add_line(&mut self, line: u32) {
        if !self.lines.contains(&line) { self.lines.push(line); }
    }
}

pub fn parse_module_blocks(tree: &SyntaxTree, scope: &str, code_content: &str) -> Vec<DataFlowBlock> {
    let module_name = get_module_name(tree).unwrap_or_else(|| "unknown".to_string());
    let mut bid_counter: u64 = 0;
    let mut blocks = Vec::new();
    let mut last_port_direction: Option<PortDirection> = None;

    for node in tree {
        match &node {
            RefNode::AnsiPortDeclarationNet(decl) => {
                let dir = get_port_direction(decl);
                if dir.is_some() { last_port_direction = dir.clone(); }
                let effective = dir.or_else(|| last_port_direction.clone());
                if let Some(pd) = effective {
                    let btype = match pd {
                        PortDirection::Input(_) => BlockType::ModuleInput,
                        PortDirection::Output(_) => BlockType::ModuleOutput,
                        _ => continue,
                    };
                    let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, btype);
                    bid_counter += 1;
                    collect_lines(tree, code_content, RefNode::AnsiPortDeclarationNet(decl), &mut |l| blk.add_line(l));
                    blocks.push(blk);
                }
            }
            RefNode::AnsiPortDeclarationVariable(decl) => {
                let dir = unwrap_node!(decl.clone(), PortDirection).and_then(|n| match n {
                    RefNode::PortDirection(d) => Some(d), _ => None,
                });
                if dir.is_some() { last_port_direction = dir.cloned(); }
                let effective = dir.cloned().or_else(|| last_port_direction.clone());
                if let Some(pd) = effective {
                    let btype = match pd {
                        PortDirection::Input(_) => BlockType::ModuleInput,
                        PortDirection::Output(_) => BlockType::ModuleOutput,
                        _ => continue,
                    };
                    let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, btype);
                    bid_counter += 1;
                    collect_lines(tree, code_content, RefNode::AnsiPortDeclarationVariable(decl), &mut |l| blk.add_line(l));
                    blocks.push(blk);
                }
            }
            RefNode::AlwaysConstruct(always_construct) => {
                let ctype = match always_construct.nodes.0 {
                    AlwaysKeyword::AlwaysComb(_) => CircuitType::COMB,
                    AlwaysKeyword::AlwaysFf(_) | AlwaysKeyword::AlwaysLatch(_) => CircuitType::SEQ,
                    AlwaysKeyword::Always(_) => CircuitType::SEQ,
                };
                let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, BlockType::Always(ctype));
                bid_counter += 1;
                collect_lines(tree, code_content, RefNode::AlwaysConstruct(always_construct), &mut |l| blk.add_line(l));
                blocks.push(blk);
            }
            RefNode::NetAssignment(assign) => {
                let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, BlockType::Assign);
                bid_counter += 1;
                collect_lines(tree, code_content, RefNode::NetAssignment(assign), &mut |l| blk.add_line(l));
                blocks.push(blk);
            }
            RefNode::NetDeclaration(net_decl) => {
                // Handle `wire x = expr;` inline initializers
                let has_init = RefNode::NetDeclaration(net_decl).into_iter()
                    .any(|child| matches!(child, RefNode::NetDeclAssignment(_)));
                if !has_init { continue; }
                for child in RefNode::NetDeclaration(net_decl).into_iter() {
                    if let RefNode::NetDeclAssignment(decl_assign) = child {
                        if decl_assign.nodes.2.is_some() {
                            let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, BlockType::Assign);
                            bid_counter += 1;
                            collect_lines(tree, code_content, RefNode::NetDeclAssignment(&decl_assign), &mut |l| blk.add_line(l));
                            blocks.push(blk);
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    blocks
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

fn get_port_direction(decl: &sv_parser::AnsiPortDeclarationNet) -> Option<PortDirection> {
    if let Some(node) = unwrap_node!(decl, PortDirection) {
        if let RefNode::PortDirection(d) = node {
            return Some(d.clone());
        }
    }
    None
}

fn collect_lines<F: FnMut(u32)>(tree: &SyntaxTree, code: &str, ref_node: RefNode, f: &mut F) {
    for child in ref_node.into_iter() {
        if let RefNode::Locate(locate) = child {
            if let Some((_, offset)) = tree.get_origin(&locate) {
                if let Some(line) = line_from_offset(code, offset) {
                    f(line);
                }
            }
        }
    }
}

fn line_from_offset(code: &str, offset: usize) -> Option<u32> {
    if offset > code.len() { return None; }
    Some((code[..offset].chars().filter(|&c| c == '\n').count() + 1) as u32)
}
