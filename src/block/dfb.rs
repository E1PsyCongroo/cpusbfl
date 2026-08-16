use sv_parser::{AlwaysKeyword, PortDirection, RefNode, SyntaxTree, unwrap_node};

use super::{Block, BlockType, CircuitType};

pub(crate) struct DataFlowBlock {
    bid: u64,
    module_name: String,
    scope: String,
    block_type: BlockType,
    lines: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SignalAssignment {
    lhs_signals: Vec<String>,
    line: u32,
    statement: String,
}

impl SignalAssignment {
    pub(crate) fn line(&self) -> u32 {
        self.line
    }

    pub(crate) fn statement(&self) -> &str {
        &self.statement
    }

    pub(crate) fn modifies(&self, signal: &str) -> bool {
        let signal = normalize_signal_name(signal);
        !signal.is_empty()
            && self
                .lhs_signals
                .iter()
                .any(|lhs| signal_names_overlap(&signal, lhs))
    }
}

impl Block for DataFlowBlock {
    fn bid(&self) -> u64 {
        self.bid
    }
    fn module_name(&self) -> &str {
        &self.module_name
    }
    fn scope(&self) -> &str {
        &self.scope
    }
    fn block_type(&self) -> &BlockType {
        &self.block_type
    }
    fn line_ranges(&self) -> Vec<u32> {
        self.lines.clone()
    }
}

impl DataFlowBlock {
    pub(super) fn new(bid: u64, module_name: &str, scope: &str, block_type: BlockType) -> Self {
        Self {
            bid,
            module_name: module_name.to_string(),
            scope: scope.to_string(),
            block_type,
            lines: Vec::new(),
        }
    }

    fn add_line(&mut self, line: u32) {
        if !self.lines.contains(&line) {
            self.lines.push(line);
        }
    }
}

pub(crate) fn parse_module_blocks(
    tree: &SyntaxTree,
    scope: &str,
    code_content: &str,
) -> Vec<DataFlowBlock> {
    let module_name = get_module_name(tree).unwrap_or_else(|| "unknown".to_string());
    let mut bid_counter: u64 = 0;
    let mut blocks = Vec::new();
    let mut last_port_direction: Option<PortDirection> = None;

    for node in tree {
        match &node {
            RefNode::AnsiPortDeclarationNet(decl) => {
                let dir = get_port_direction(decl);
                if dir.is_some() {
                    last_port_direction = dir.clone();
                }
                let effective = dir.or_else(|| last_port_direction.clone());
                if let Some(pd) = effective {
                    let btype = match pd {
                        PortDirection::Input(_) => BlockType::ModuleInput,
                        PortDirection::Output(_) => BlockType::ModuleOutput,
                        _ => continue,
                    };
                    let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, btype);
                    bid_counter += 1;
                    collect_lines(
                        tree,
                        code_content,
                        RefNode::AnsiPortDeclarationNet(decl),
                        &mut |l| blk.add_line(l),
                    );
                    blocks.push(blk);
                }
            }
            RefNode::AnsiPortDeclarationVariable(decl) => {
                let dir = unwrap_node!(decl.clone(), PortDirection).and_then(|n| match n {
                    RefNode::PortDirection(d) => Some(d),
                    _ => None,
                });
                if dir.is_some() {
                    last_port_direction = dir.cloned();
                }
                let effective = dir.cloned().or_else(|| last_port_direction.clone());
                if let Some(pd) = effective {
                    let btype = match pd {
                        PortDirection::Input(_) => BlockType::ModuleInput,
                        PortDirection::Output(_) => BlockType::ModuleOutput,
                        _ => continue,
                    };
                    let mut blk = DataFlowBlock::new(bid_counter, &module_name, scope, btype);
                    bid_counter += 1;
                    collect_lines(
                        tree,
                        code_content,
                        RefNode::AnsiPortDeclarationVariable(decl),
                        &mut |l| blk.add_line(l),
                    );
                    blocks.push(blk);
                }
            }
            RefNode::AlwaysConstruct(always_construct) => {
                let ctype = match always_construct.nodes.0 {
                    AlwaysKeyword::AlwaysComb(_) => CircuitType::COMB,
                    AlwaysKeyword::AlwaysFf(_) | AlwaysKeyword::AlwaysLatch(_) => CircuitType::SEQ,
                    AlwaysKeyword::Always(_) => CircuitType::SEQ,
                };
                let mut blk =
                    DataFlowBlock::new(bid_counter, &module_name, scope, BlockType::Always(ctype));
                bid_counter += 1;
                collect_lines(
                    tree,
                    code_content,
                    RefNode::AlwaysConstruct(always_construct),
                    &mut |l| blk.add_line(l),
                );
                blocks.push(blk);
            }
            RefNode::NetAssignment(assign) => {
                let mut blk =
                    DataFlowBlock::new(bid_counter, &module_name, scope, BlockType::Assign);
                bid_counter += 1;
                collect_lines(
                    tree,
                    code_content,
                    RefNode::NetAssignment(assign),
                    &mut |l| blk.add_line(l),
                );
                blocks.push(blk);
            }
            RefNode::NetDeclaration(net_decl) => {
                // Handle `wire x = expr;` inline initializers
                let has_init = RefNode::NetDeclaration(net_decl)
                    .into_iter()
                    .any(|child| matches!(child, RefNode::NetDeclAssignment(_)));
                if !has_init {
                    continue;
                }
                for child in RefNode::NetDeclaration(net_decl).into_iter() {
                    if let RefNode::NetDeclAssignment(decl_assign) = child {
                        if decl_assign.nodes.2.is_some() {
                            let mut blk = DataFlowBlock::new(
                                bid_counter,
                                &module_name,
                                scope,
                                BlockType::Assign,
                            );
                            bid_counter += 1;
                            collect_lines(
                                tree,
                                code_content,
                                RefNode::NetDeclAssignment(&decl_assign),
                                &mut |l| blk.add_line(l),
                            );
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

pub(crate) fn parse_signal_assignments(
    tree: &SyntaxTree,
    code_content: &str,
) -> Vec<SignalAssignment> {
    let mut assignments = Vec::new();

    for node in tree {
        let (statement, lhs, statement_text) = match node {
            RefNode::BlockingAssignmentVariable(assignment) => (
                RefNode::BlockingAssignmentVariable(assignment),
                RefNode::VariableLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::BlockingAssignmentNonrangeVariable(assignment) => (
                RefNode::BlockingAssignmentNonrangeVariable(assignment),
                RefNode::NonrangeVariableLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::BlockingAssignmentHierarchicalVariable(assignment) => (
                RefNode::BlockingAssignmentHierarchicalVariable(assignment),
                RefNode::HierarchicalVariableIdentifier(&assignment.nodes.1),
                tree.get_str_trim(assignment),
            ),
            RefNode::OperatorAssignment(assignment) => (
                RefNode::OperatorAssignment(assignment),
                RefNode::VariableLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::NonblockingAssignment(assignment) => (
                RefNode::NonblockingAssignment(assignment),
                RefNode::VariableLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::NetAssignment(assignment) => (
                RefNode::NetAssignment(assignment),
                RefNode::NetLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::VariableAssignment(assignment) => (
                RefNode::VariableAssignment(assignment),
                RefNode::VariableLvalue(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::NetDeclAssignment(assignment) if assignment.nodes.2.is_some() => (
                RefNode::NetDeclAssignment(assignment),
                RefNode::NetIdentifier(&assignment.nodes.0),
                tree.get_str_trim(assignment),
            ),
            RefNode::VariableDeclAssignmentVariable(assignment) if assignment.nodes.2.is_some() => {
                (
                    RefNode::VariableDeclAssignmentVariable(assignment),
                    RefNode::VariableIdentifier(&assignment.nodes.0),
                    tree.get_str_trim(assignment),
                )
            }
            _ => continue,
        };

        let lhs_signals = collect_lhs_signals(tree, lhs);
        let Some(line) = first_origin_line(tree, code_content, statement.clone()) else {
            continue;
        };
        let Some(statement_text) = statement_text else {
            continue;
        };
        let statement_text = statement_text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        if !lhs_signals.is_empty() && !statement_text.is_empty() {
            let assignment = SignalAssignment {
                lhs_signals,
                line,
                statement: statement_text,
            };
            if !assignments.contains(&assignment) {
                assignments.push(assignment);
            }
        }
    }

    assignments
}

fn collect_lhs_signals(tree: &SyntaxTree, lhs: RefNode<'_>) -> Vec<String> {
    let mut signals = Vec::new();

    for node in lhs.into_iter() {
        let name = match node {
            RefNode::HierarchicalIdentifier(identifier) => tree.get_str(identifier),
            RefNode::NetIdentifier(identifier) => tree.get_str(identifier),
            RefNode::VariableIdentifier(identifier) => tree.get_str(identifier),
            _ => None,
        };

        if let Some(name) = name {
            let name = normalize_signal_name(name);
            if !name.is_empty() && !signals.contains(&name) {
                signals.push(name);
            }
        }
    }

    signals
}

fn normalize_signal_name(signal: &str) -> String {
    let mut normalized = String::new();
    let mut bracket_depth = 0_u32;

    for ch in signal.trim().chars() {
        match ch {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            _ if bracket_depth == 0 && !ch.is_whitespace() => normalized.push(ch),
            _ => {}
        }
    }

    normalized
}

fn signal_names_overlap(target: &str, lhs: &str) -> bool {
    target == lhs
        || target
            .strip_prefix(lhs)
            .is_some_and(|suffix| suffix.starts_with('.'))
        || lhs
            .strip_prefix(target)
            .is_some_and(|suffix| suffix.starts_with('.'))
        // Verilator puts named generate-block prefixes in the toggle comment,
        // while an assignment inside that block only contains the local name.
        || target.ends_with(&format!(".{lhs}"))
}

fn first_origin_line(tree: &SyntaxTree, code: &str, ref_node: RefNode<'_>) -> Option<u32> {
    ref_node.into_iter().find_map(|child| {
        if let RefNode::Locate(locate) = child {
            tree.get_origin(&locate)
                .and_then(|(_, offset)| line_from_offset(code, offset))
        } else {
            None
        }
    })
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
    if offset > code.len() {
        return None;
    }
    Some((code[..offset].chars().filter(|&c| c == '\n').count() + 1) as u32)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs};

    use sv_parser::{Define, parse_sv};

    use super::*;

    #[test]
    fn finds_assignments_that_modify_toggle_signal() {
        let code = r#"module top(input logic clk_i);
  logic flag;
  logic [3:0] data;
  assign flag = data[0];
  always_comb begin
    flag = data[1];
    data[2] <= flag;
    data += 1'b1;
  end
endmodule
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("top.sv");
        fs::write(&path, code).unwrap();
        let defines: HashMap<String, Option<Define>> = HashMap::new();
        let (tree, _) =
            parse_sv(&path, &defines, &[] as &[&std::path::Path], false, false).unwrap();

        let assignments = parse_signal_assignments(&tree, code);
        let flag_lines = assignments
            .iter()
            .filter(|assignment| assignment.modifies("flag"))
            .map(SignalAssignment::line)
            .collect::<Vec<_>>();
        let data_lines = assignments
            .iter()
            .filter(|assignment| assignment.modifies("data[3]"))
            .map(SignalAssignment::line)
            .collect::<Vec<_>>();

        assert_eq!(flag_lines, vec![4, 6]);
        assert_eq!(data_lines, vec![7, 8]);
    }

    #[test]
    fn matches_aggregate_and_generate_qualified_toggle_names() {
        assert!(signal_names_overlap(
            "crash_dump_o.exception_pc",
            "crash_dump_o"
        ));
        assert!(signal_names_overlap(
            "g_named.unused_signal",
            "unused_signal"
        ));
        assert!(!signal_names_overlap("load_err_q", "load_err_d"));
        assert_eq!(normalize_signal_name("array[0][3]"), "array");
    }
}
