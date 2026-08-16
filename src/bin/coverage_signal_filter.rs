//! Find RTL signals whose toggle coverage can be disabled.
//!
//! The analysis is deliberately conservative: module inputs are always reported,
//! while other declared signals are reported only when they are outside the
//! backward dependency cone of every module output.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    error::Error,
    fs, io,
    path::{Path, PathBuf},
};

use clap::Parser;
use serde::Serialize;
use sv_parser::{
    Define, ListOfPortConnections, LoopStatement, NamedPortConnection, NodeEvent, PortDirection,
    RefNode, RefNodes, SyntaxTree, parse_sv, unwrap_node,
};

#[derive(Debug, Parser)]
#[command(
    name = "coverage-signal-filter",
    about = "List module inputs and signals unrelated to module outputs"
)]
struct Args {
    /// RTL source file or directory. Directories are scanned recursively for .sv and .v files.
    #[arg(long, visible_alias = "rtl_path", value_name = "PATH")]
    rtl_path: PathBuf,

    /// SystemVerilog include directory. May be repeated.
    #[arg(
        long = "include-path",
        visible_aliases = ["include_paths", "include-paths"],
        value_name = "PATH"
    )]
    include_paths: Vec<PathBuf>,

    /// Name of the design's top module.
    #[arg(long, visible_alias = "top_module")]
    top_module: String,

    /// Hierarchical scope assigned to the top module in the report.
    #[arg(long, visible_alias = "top_scope")]
    top_scope: String,

    /// Write JSON to this file instead of stdout.
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SignalKind {
    Input,
    Output,
    Inout,
    Internal,
}

#[derive(Clone, Debug)]
struct Signal {
    kind: SignalKind,
    lines: BTreeSet<u32>,
}

#[derive(Clone, Debug, Default)]
struct Connection {
    port_name: Option<String>,
    expression_signals: BTreeSet<String>,
    implicit: bool,
}

#[derive(Clone, Debug)]
struct Instance {
    module_name: String,
    instance_name: String,
    generate_prefix: Vec<String>,
    connections: Vec<Connection>,
}

#[derive(Debug)]
struct ModuleAnalysis {
    name: String,
    source: PathBuf,
    signals: BTreeMap<String, Signal>,
    port_order: Vec<String>,
    inputs: BTreeSet<String>,
    outputs: BTreeSet<String>,
    inouts: BTreeSet<String>,
    dependencies: BTreeMap<String, BTreeSet<String>>,
    instances: Vec<Instance>,
}

#[derive(Debug, Serialize)]
struct Report {
    top_module: String,
    top_scope: String,
    rtl_path: PathBuf,
    modules: Vec<ModuleReport>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ModuleReport {
    module: String,
    source: PathBuf,
    scopes: Vec<String>,
    signals: Vec<ExcludedSignal>,
}

#[derive(Debug, Serialize)]
struct ExcludedSignal {
    name: String,
    lines: Vec<u32>,
    reason: ExclusionReason,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExclusionReason {
    ModuleInput,
    NotRelatedToModuleOutput,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let report = analyze_design(
        &args.rtl_path,
        &args.include_paths,
        &args.top_module,
        &args.top_scope,
    )?;
    let json = serde_json::to_string_pretty(&report)?;

    if let Some(output) = args.output {
        if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        fs::write(output, format!("{json}\n"))?;
    } else {
        println!("{json}");
    }
    Ok(())
}

fn analyze_design(
    rtl_path: &Path,
    include_paths: &[PathBuf],
    top_module: &str,
    top_scope: &str,
) -> Result<Report, Box<dyn Error>> {
    let rtl_files = collect_rtl_files(rtl_path)?;
    if rtl_files.is_empty() {
        return Err(invalid_input(format!(
            "no .sv or .v files found under {}",
            rtl_path.display()
        )));
    }

    let defines: HashMap<String, Option<Define>> = HashMap::new();
    let mut modules = BTreeMap::new();
    let mut warnings = Vec::new();

    for file in rtl_files {
        match parse_sv(&file, &defines, include_paths, false, false) {
            Ok((tree, _)) => {
                for module in analyze_modules_in_tree(&tree, &file)? {
                    if let Some(previous) = modules.insert(module.name.clone(), module) {
                        return Err(invalid_input(format!(
                            "module '{}' is defined by both {} and {}",
                            previous.name,
                            previous.source.display(),
                            file.display()
                        )));
                    }
                }
            }
            Err(error) => warnings.push(format!("failed to parse {}: {error:?}", file.display())),
        }
    }

    if !modules.contains_key(top_module) {
        let available = modules.keys().cloned().collect::<Vec<_>>().join(", ");
        let parse_diagnostics = if warnings.is_empty() {
            String::new()
        } else {
            format!("; parse diagnostics:\n{}", warnings.join("\n"))
        };
        return Err(invalid_input(format!(
            "top module '{top_module}' was not found; available modules: {available}{parse_diagnostics}"
        )));
    }

    propagate_instance_dependencies(&mut modules);
    let reachable = reachable_module_scopes(&modules, top_module, top_scope, &mut warnings);
    let mut module_reports = Vec::new();

    for (module_name, scopes) in reachable {
        let module = &modules[&module_name];
        let output_cone = backward_closure(
            module.outputs.iter().chain(module.inouts.iter()).cloned(),
            &module.dependencies,
        );
        let mut signals = Vec::new();

        for (name, signal) in &module.signals {
            let reason = match signal.kind {
                SignalKind::Input => Some(ExclusionReason::ModuleInput),
                SignalKind::Output | SignalKind::Inout => None,
                SignalKind::Internal if !output_cone.contains(name) => {
                    Some(ExclusionReason::NotRelatedToModuleOutput)
                }
                SignalKind::Internal => None,
            };
            if let Some(reason) = reason {
                signals.push(ExcludedSignal {
                    name: name.clone(),
                    lines: signal.lines.iter().copied().collect(),
                    reason,
                });
            }
        }

        module_reports.push(ModuleReport {
            module: module_name,
            source: module.source.clone(),
            scopes: scopes.into_iter().collect(),
            signals,
        });
    }

    Ok(Report {
        top_module: top_module.to_owned(),
        top_scope: top_scope.to_owned(),
        rtl_path: rtl_path.to_path_buf(),
        modules: module_reports,
        warnings,
    })
}

fn collect_rtl_files(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rtl_files_inner(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_rtl_files_inner(path: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    if path.is_file() {
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("sv" | "v")
        ) {
            files.push(path.to_path_buf());
        }
        return Ok(());
    }
    if !path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("RTL path does not exist: {}", path.display()),
        ));
    }

    let mut entries = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        collect_rtl_files_inner(&entry.path(), files)?;
    }
    Ok(())
}

fn analyze_modules_in_tree(
    tree: &SyntaxTree,
    source: &Path,
) -> Result<Vec<ModuleAnalysis>, Box<dyn Error>> {
    let mut analyses = Vec::new();
    for node in tree {
        let root = match node {
            RefNode::ModuleDeclarationAnsi(module) => RefNode::ModuleDeclarationAnsi(module),
            RefNode::ModuleDeclarationNonansi(module) => RefNode::ModuleDeclarationNonansi(module),
            RefNode::ModuleDeclarationWildcard(module) => {
                RefNode::ModuleDeclarationWildcard(module)
            }
            _ => continue,
        };
        analyses.push(analyze_module(tree, root, source)?);
    }
    Ok(analyses)
}

fn analyze_module(
    tree: &SyntaxTree,
    root: RefNode<'_>,
    source: &Path,
) -> Result<ModuleAnalysis, Box<dyn Error>> {
    let name = module_name(tree, root.clone())
        .ok_or_else(|| invalid_input(format!("unnamed module in {}", source.display())))?;
    let source_text = fs::read_to_string(source)?;
    let mut signals = BTreeMap::new();
    let mut port_order = Vec::new();
    let mut inputs = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    let mut inouts = BTreeSet::new();
    let mut last_port_direction = None;
    let mut functions = BTreeSet::new();

    for node in root.clone().into_iter() {
        match node {
            RefNode::AnsiPortDeclarationNet(decl) => {
                let explicit = unwrap_node!(decl, PortDirection).and_then(|node| match node {
                    RefNode::PortDirection(direction) => Some(direction.clone()),
                    _ => None,
                });
                if explicit.is_some() {
                    last_port_direction = explicit.clone();
                }
                if let Some(direction) = explicit.or_else(|| last_port_direction.clone()) {
                    let port_name = node_text(tree, RefNode::PortIdentifier(&decl.nodes.1));
                    if let Some(port_name) = port_name {
                        add_port(
                            &mut signals,
                            &mut port_order,
                            &mut inputs,
                            &mut outputs,
                            &mut inouts,
                            port_name,
                            direction,
                            origin_line(tree, &source_text, node.clone()),
                        );
                    }
                }
            }
            RefNode::AnsiPortDeclarationVariable(decl) => {
                let explicit = unwrap_node!(decl, PortDirection).and_then(|node| match node {
                    RefNode::PortDirection(direction) => Some(direction.clone()),
                    _ => None,
                });
                if explicit.is_some() {
                    last_port_direction = explicit.clone();
                }
                if let Some(direction) = explicit.or_else(|| last_port_direction.clone()) {
                    let port_name = node_text(tree, RefNode::PortIdentifier(&decl.nodes.1));
                    if let Some(port_name) = port_name {
                        add_port(
                            &mut signals,
                            &mut port_order,
                            &mut inputs,
                            &mut outputs,
                            &mut inouts,
                            port_name,
                            direction,
                            origin_line(tree, &source_text, node.clone()),
                        );
                    }
                }
            }
            RefNode::AnsiPortDeclarationParen(decl) => {
                if let Some(direction) = decl.nodes.0.clone() {
                    last_port_direction = Some(direction.clone());
                    if let Some(port_name) = node_text(tree, RefNode::PortIdentifier(&decl.nodes.2))
                    {
                        add_port(
                            &mut signals,
                            &mut port_order,
                            &mut inputs,
                            &mut outputs,
                            &mut inouts,
                            port_name,
                            direction,
                            origin_line(tree, &source_text, node.clone()),
                        );
                    }
                }
            }
            RefNode::PortDeclarationInput(decl) => {
                add_nonansi_ports(
                    tree,
                    node.clone(),
                    RefNode::InputDeclaration(&decl.nodes.1),
                    SignalKind::Input,
                    &source_text,
                    &mut signals,
                    &mut port_order,
                    &mut inputs,
                    &mut outputs,
                    &mut inouts,
                );
            }
            RefNode::PortDeclarationOutput(decl) => {
                add_nonansi_ports(
                    tree,
                    node.clone(),
                    RefNode::OutputDeclaration(&decl.nodes.1),
                    SignalKind::Output,
                    &source_text,
                    &mut signals,
                    &mut port_order,
                    &mut inputs,
                    &mut outputs,
                    &mut inouts,
                );
            }
            RefNode::PortDeclarationInout(decl) => {
                add_nonansi_ports(
                    tree,
                    node.clone(),
                    RefNode::InoutDeclaration(&decl.nodes.1),
                    SignalKind::Inout,
                    &source_text,
                    &mut signals,
                    &mut port_order,
                    &mut inputs,
                    &mut outputs,
                    &mut inouts,
                );
            }
            RefNode::VariableDeclAssignmentVariable(decl) => {
                if let Some(signal_name) =
                    node_text(tree, RefNode::VariableIdentifier(&decl.nodes.0))
                {
                    add_signal(
                        &mut signals,
                        signal_name,
                        SignalKind::Internal,
                        origin_line(tree, &source_text, node.clone()),
                    );
                }
            }
            RefNode::NetDeclAssignment(decl) => {
                if let Some(signal_name) = node_text(tree, RefNode::NetIdentifier(&decl.nodes.0)) {
                    add_signal(
                        &mut signals,
                        signal_name,
                        SignalKind::Internal,
                        origin_line(tree, &source_text, node.clone()),
                    );
                }
            }
            RefNode::FunctionDeclaration(decl) => {
                if let Some(RefNode::FunctionIdentifier(identifier)) =
                    unwrap_node!(decl, FunctionIdentifier)
                    && let Some(function_name) = tree.get_str(&identifier.nodes.0)
                {
                    functions.insert(function_name.trim().to_owned());
                }
            }
            _ => {}
        }
    }

    // Port declarations also appear as variable/net declaration assignments in
    // the syntax tree. Restore the stronger port kind after collecting internals.
    for input in &inputs {
        signals.get_mut(input).expect("input was inserted").kind = SignalKind::Input;
    }
    for output in &outputs {
        signals.get_mut(output).expect("output was inserted").kind = SignalKind::Output;
    }
    for inout in &inouts {
        signals.get_mut(inout).expect("inout was inserted").kind = SignalKind::Inout;
    }

    let allowed_names = signals
        .keys()
        .chain(functions.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let dependencies = collect_dependencies(tree, root.clone(), &allowed_names, &functions);
    let instances = collect_instances(tree, root);

    Ok(ModuleAnalysis {
        name,
        source: source.to_path_buf(),
        signals,
        port_order,
        inputs,
        outputs,
        inouts,
        dependencies,
        instances,
    })
}

#[allow(clippy::too_many_arguments)]
fn add_port(
    signals: &mut BTreeMap<String, Signal>,
    port_order: &mut Vec<String>,
    inputs: &mut BTreeSet<String>,
    outputs: &mut BTreeSet<String>,
    inouts: &mut BTreeSet<String>,
    name: String,
    direction: PortDirection,
    line: Option<u32>,
) {
    let kind = match direction {
        PortDirection::Input(_) => SignalKind::Input,
        PortDirection::Output(_) => SignalKind::Output,
        PortDirection::Inout(_) | PortDirection::Ref(_) => SignalKind::Inout,
    };
    add_port_with_kind(
        signals, port_order, inputs, outputs, inouts, name, kind, line,
    );
}

#[allow(clippy::too_many_arguments)]
fn add_nonansi_ports(
    tree: &SyntaxTree,
    declaration: RefNode<'_>,
    identifiers_root: RefNode<'_>,
    kind: SignalKind,
    source_text: &str,
    signals: &mut BTreeMap<String, Signal>,
    port_order: &mut Vec<String>,
    inputs: &mut BTreeSet<String>,
    outputs: &mut BTreeSet<String>,
    inouts: &mut BTreeSet<String>,
) {
    let line = origin_line(tree, source_text, declaration);
    for child in identifiers_root.into_iter() {
        let name = match child {
            RefNode::PortIdentifier(identifier) => tree.get_str(&identifier.nodes.0),
            RefNode::VariableIdentifier(identifier) => tree.get_str(&identifier.nodes.0),
            _ => None,
        };
        if let Some(name) = name {
            add_port_with_kind(
                signals,
                port_order,
                inputs,
                outputs,
                inouts,
                name.trim().to_owned(),
                kind,
                line,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_port_with_kind(
    signals: &mut BTreeMap<String, Signal>,
    port_order: &mut Vec<String>,
    inputs: &mut BTreeSet<String>,
    outputs: &mut BTreeSet<String>,
    inouts: &mut BTreeSet<String>,
    name: String,
    kind: SignalKind,
    line: Option<u32>,
) {
    if !port_order.contains(&name) {
        port_order.push(name.clone());
    }
    match kind {
        SignalKind::Input => {
            inputs.insert(name.clone());
        }
        SignalKind::Output => {
            outputs.insert(name.clone());
        }
        SignalKind::Inout => {
            inouts.insert(name.clone());
        }
        SignalKind::Internal => {}
    }
    add_signal(signals, name, kind, line);
}

fn add_signal(
    signals: &mut BTreeMap<String, Signal>,
    name: String,
    kind: SignalKind,
    line: Option<u32>,
) {
    let entry = signals.entry(name).or_insert_with(|| Signal {
        kind,
        lines: BTreeSet::new(),
    });
    if kind != SignalKind::Internal {
        entry.kind = kind;
    }
    if let Some(line) = line {
        entry.lines.insert(line);
    }
}

fn collect_dependencies(
    tree: &SyntaxTree,
    root: RefNode<'_>,
    allowed_names: &BTreeSet<String>,
    functions: &BTreeSet<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut controls: Vec<BTreeSet<String>> = Vec::new();
    let mut function_stack: Vec<String> = Vec::new();

    for event in root.into_iter().event() {
        match event {
            NodeEvent::Enter(RefNode::ConditionalStatement(statement)) => {
                let mut names =
                    identifiers_in_node(tree, RefNode::CondPredicate(&statement.nodes.2.nodes.1));
                for (_, _, predicate, _) in &statement.nodes.4 {
                    names.extend(identifiers_in_node(
                        tree,
                        RefNode::CondPredicate(&predicate.nodes.1),
                    ));
                }
                retain_allowed(&mut names, allowed_names);
                controls.push(names);
            }
            NodeEvent::Leave(RefNode::ConditionalStatement(_)) => {
                controls.pop();
            }
            NodeEvent::Enter(RefNode::CaseStatement(statement)) => {
                let mut names = BTreeSet::new();
                for child in RefNode::CaseStatement(statement).into_iter() {
                    match child {
                        RefNode::CaseExpression(expression) => {
                            names.extend(identifiers_in_node(
                                tree,
                                RefNode::CaseExpression(expression),
                            ));
                        }
                        RefNode::CaseItemExpression(expression) => {
                            names.extend(identifiers_in_node(
                                tree,
                                RefNode::CaseItemExpression(expression),
                            ));
                        }
                        _ => {}
                    }
                }
                retain_allowed(&mut names, allowed_names);
                controls.push(names);
            }
            NodeEvent::Leave(RefNode::CaseStatement(_)) => {
                controls.pop();
            }
            NodeEvent::Enter(RefNode::LoopStatement(statement)) => {
                let mut names = loop_control_identifiers(tree, statement);
                retain_allowed(&mut names, allowed_names);
                controls.push(names);
            }
            NodeEvent::Leave(RefNode::LoopStatement(_)) => {
                controls.pop();
            }
            NodeEvent::Enter(RefNode::FunctionDeclaration(declaration)) => {
                let function = unwrap_node!(declaration, FunctionIdentifier).and_then(|node| {
                    if let RefNode::FunctionIdentifier(identifier) = node {
                        tree.get_str(&identifier.nodes.0)
                            .map(|name| name.trim().to_owned())
                    } else {
                        None
                    }
                });
                if let Some(function) = function {
                    function_stack.push(function);
                }
            }
            NodeEvent::Leave(RefNode::FunctionDeclaration(_)) => {
                function_stack.pop();
            }
            NodeEvent::Enter(RefNode::JumpStatementReturn(statement)) => {
                if let Some(function) = function_stack.last() {
                    let mut rhs =
                        identifiers_in_node(tree, RefNode::JumpStatementReturn(statement));
                    retain_allowed(&mut rhs, allowed_names);
                    add_control_dependencies(&mut rhs, &controls);
                    add_dependency_edges(&mut dependencies, [function.clone()], rhs);
                }
            }
            NodeEvent::Enter(node) => {
                if let Some((lhs_node, whole_statement)) = assignment_parts(node) {
                    let lhs = lhs_signals(tree, lhs_node)
                        .into_iter()
                        .filter(|name| allowed_names.contains(name))
                        .collect::<BTreeSet<_>>();
                    let mut rhs = identifiers_in_node(tree, whole_statement);
                    retain_allowed(&mut rhs, allowed_names);
                    for lhs_name in &lhs {
                        rhs.remove(lhs_name);
                    }
                    add_control_dependencies(&mut rhs, &controls);
                    add_dependency_edges(&mut dependencies, lhs, rhs);
                }
            }
            NodeEvent::Leave(_) => {}
        }
    }

    // A function call is represented by the function identifier in the RHS.
    // Keeping pseudo function nodes in the graph lets return/function-name
    // assignments connect the call to its implementation dependencies.
    for function in functions {
        dependencies.entry(function.clone()).or_default();
    }
    dependencies
}

fn loop_control_identifiers(tree: &SyntaxTree, statement: &LoopStatement) -> BTreeSet<String> {
    match statement {
        LoopStatement::Forever(_) => BTreeSet::new(),
        LoopStatement::Repeat(statement) => {
            identifiers_in_node(tree, RefNode::Expression(&statement.nodes.1.nodes.1))
        }
        LoopStatement::While(statement) => {
            identifiers_in_node(tree, RefNode::Expression(&statement.nodes.1.nodes.1))
        }
        LoopStatement::For(statement) => statement
            .nodes
            .1
            .nodes
            .1
            .2
            .as_ref()
            .map(|expression| identifiers_in_node(tree, RefNode::Expression(expression)))
            .unwrap_or_default(),
        LoopStatement::DoWhile(statement) => {
            identifiers_in_node(tree, RefNode::Expression(&statement.nodes.3.nodes.1))
        }
        LoopStatement::Foreach(statement) => identifiers_in_node(
            tree,
            RefNode::PsOrHierarchicalArrayIdentifier(&statement.nodes.1.nodes.1.0),
        ),
    }
}

fn assignment_parts(node: RefNode<'_>) -> Option<(RefNode<'_>, RefNode<'_>)> {
    match node {
        RefNode::BlockingAssignmentVariable(assignment) => Some((
            RefNode::VariableLvalue(&assignment.nodes.0),
            RefNode::BlockingAssignmentVariable(assignment),
        )),
        RefNode::BlockingAssignmentNonrangeVariable(assignment) => Some((
            RefNode::NonrangeVariableLvalue(&assignment.nodes.0),
            RefNode::BlockingAssignmentNonrangeVariable(assignment),
        )),
        RefNode::BlockingAssignmentHierarchicalVariable(assignment) => Some((
            RefNode::HierarchicalVariableIdentifier(&assignment.nodes.1),
            RefNode::BlockingAssignmentHierarchicalVariable(assignment),
        )),
        RefNode::OperatorAssignment(assignment) => Some((
            RefNode::VariableLvalue(&assignment.nodes.0),
            RefNode::OperatorAssignment(assignment),
        )),
        RefNode::NonblockingAssignment(assignment) => Some((
            RefNode::VariableLvalue(&assignment.nodes.0),
            RefNode::NonblockingAssignment(assignment),
        )),
        RefNode::NetAssignment(assignment) => Some((
            RefNode::NetLvalue(&assignment.nodes.0),
            RefNode::NetAssignment(assignment),
        )),
        RefNode::VariableAssignment(assignment) => Some((
            RefNode::VariableLvalue(&assignment.nodes.0),
            RefNode::VariableAssignment(assignment),
        )),
        RefNode::NetDeclAssignment(assignment) if assignment.nodes.2.is_some() => Some((
            RefNode::NetIdentifier(&assignment.nodes.0),
            RefNode::NetDeclAssignment(assignment),
        )),
        RefNode::VariableDeclAssignmentVariable(assignment) if assignment.nodes.2.is_some() => {
            Some((
                RefNode::VariableIdentifier(&assignment.nodes.0),
                RefNode::VariableDeclAssignmentVariable(assignment),
            ))
        }
        _ => None,
    }
}

fn add_control_dependencies(rhs: &mut BTreeSet<String>, controls: &[BTreeSet<String>]) {
    for control in controls {
        rhs.extend(control.iter().cloned());
    }
}

fn add_dependency_edges(
    dependencies: &mut BTreeMap<String, BTreeSet<String>>,
    lhs: impl IntoIterator<Item = String>,
    rhs: BTreeSet<String>,
) {
    for lhs_name in lhs {
        dependencies
            .entry(lhs_name)
            .or_default()
            .extend(rhs.iter().cloned());
    }
}

fn collect_instances(tree: &SyntaxTree, root: RefNode<'_>) -> Vec<Instance> {
    let mut instances = Vec::new();
    let mut generate_prefix = Vec::new();

    for event in root.into_iter().event() {
        match event {
            NodeEvent::Enter(RefNode::GenerateBlockIdentifier(identifier)) => {
                if let Some(name) = tree.get_str(&identifier.nodes.0) {
                    generate_prefix.push(name.trim().to_owned());
                }
            }
            NodeEvent::Leave(RefNode::GenerateBlock(_)) => {
                generate_prefix.pop();
            }
            NodeEvent::Enter(RefNode::ModuleInstantiation(instantiation)) => {
                let Some(module_name) = tree
                    .get_str(&instantiation.nodes.0.nodes.0)
                    .map(|name| name.trim().to_owned())
                else {
                    continue;
                };
                for hierarchical in instantiation.nodes.2.contents() {
                    let Some(instance_name) = tree
                        .get_str(&hierarchical.nodes.0.nodes.0.nodes.0)
                        .map(|name| name.trim().to_owned())
                    else {
                        continue;
                    };
                    instances.push(Instance {
                        module_name: module_name.clone(),
                        instance_name,
                        generate_prefix: generate_prefix.clone(),
                        connections: parse_connections(tree, &hierarchical.nodes.1.nodes.1),
                    });
                }
            }
            _ => {}
        }
    }
    instances
}

fn parse_connections(
    tree: &SyntaxTree,
    connections: &Option<ListOfPortConnections>,
) -> Vec<Connection> {
    let Some(connections) = connections else {
        return Vec::new();
    };
    match connections {
        ListOfPortConnections::Ordered(ordered) => ordered
            .nodes
            .0
            .contents()
            .into_iter()
            .map(|connection| Connection {
                port_name: None,
                expression_signals: connection
                    .nodes
                    .1
                    .as_ref()
                    .map(|expression| identifiers_in_node(tree, RefNode::Expression(expression)))
                    .unwrap_or_default(),
                implicit: false,
            })
            .collect(),
        ListOfPortConnections::Named(named) => named
            .nodes
            .0
            .contents()
            .into_iter()
            .filter_map(|connection| match connection {
                NamedPortConnection::Identifier(connection) => {
                    let port_name = tree
                        .get_str(&connection.nodes.2.nodes.0)
                        .map(|name| name.trim().to_owned())?;
                    let expression_signals = connection
                        .nodes
                        .3
                        .as_ref()
                        .and_then(|paren| paren.nodes.1.as_ref())
                        .map(|expression| {
                            identifiers_in_node(tree, RefNode::Expression(expression))
                        })
                        .unwrap_or_default();
                    Some(Connection {
                        port_name: Some(port_name),
                        expression_signals,
                        implicit: connection.nodes.3.is_none(),
                    })
                }
                // `.*` is expanded later using the child module's port list.
                NamedPortConnection::Asterisk(_) => Some(Connection {
                    port_name: None,
                    expression_signals: BTreeSet::new(),
                    implicit: true,
                }),
            })
            .collect(),
    }
}

fn propagate_instance_dependencies(modules: &mut BTreeMap<String, ModuleAnalysis>) {
    loop {
        let snapshot = modules
            .iter()
            .map(|(name, module)| (name.clone(), module.dependencies.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut additions: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();

        for (parent_name, parent) in modules.iter() {
            for instance in &parent.instances {
                let connections =
                    resolved_connections(instance, modules.get(&instance.module_name));
                if let Some(child) = modules.get(&instance.module_name) {
                    let child_dependencies = &snapshot[&child.name];
                    for child_output in child.outputs.iter().chain(child.inouts.iter()) {
                        let child_cone = backward_closure(
                            std::iter::once(child_output.clone()),
                            child_dependencies,
                        );
                        let sinks = connections.get(child_output).cloned().unwrap_or_default();
                        for child_input in child.inputs.iter().chain(child.inouts.iter()) {
                            if !child_cone.contains(child_input) {
                                continue;
                            }
                            let sources = connections.get(child_input).cloned().unwrap_or_default();
                            for sink in &sinks {
                                for source in &sources {
                                    additions
                                        .entry(parent_name.clone())
                                        .or_default()
                                        .push((sink.clone(), source.clone()));
                                }
                            }
                        }
                    }
                } else {
                    // Unknown/black-box module: assume every connected signal can
                    // affect every other connected signal. This may keep extra
                    // coverage, but cannot incorrectly suppress a real dependency.
                    let all = connections
                        .values()
                        .flat_map(|signals| signals.iter().cloned())
                        .collect::<BTreeSet<_>>();
                    for sink in &all {
                        for source in &all {
                            if sink != source {
                                additions
                                    .entry(parent_name.clone())
                                    .or_default()
                                    .push((sink.clone(), source.clone()));
                            }
                        }
                    }
                }
            }
        }

        let mut changed = false;
        for (module_name, edges) in additions {
            let module = modules.get_mut(&module_name).expect("module exists");
            for (sink, source) in edges {
                if module.signals.contains_key(&sink)
                    && module.signals.contains_key(&source)
                    && module.dependencies.entry(sink).or_default().insert(source)
                {
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

fn resolved_connections(
    instance: &Instance,
    child: Option<&ModuleAnalysis>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut result: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (index, connection) in instance.connections.iter().enumerate() {
        if connection.port_name.is_none() && connection.implicit {
            if let Some(child) = child {
                for port in &child.port_order {
                    result.entry(port.clone()).or_default().insert(port.clone());
                }
            }
            continue;
        }
        let port_name = connection
            .port_name
            .clone()
            .or_else(|| child.and_then(|module| module.port_order.get(index).cloned()))
            .unwrap_or_else(|| format!("#{index}"));
        let mut signal_names = connection.expression_signals.clone();
        if connection.implicit && signal_names.is_empty() {
            signal_names.insert(port_name.clone());
        }
        result.entry(port_name).or_default().extend(signal_names);
    }
    result
}

fn reachable_module_scopes(
    modules: &BTreeMap<String, ModuleAnalysis>,
    top_module: &str,
    top_scope: &str,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut scopes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([(
        top_module.to_owned(),
        top_scope.to_owned(),
        vec![top_module.to_owned()],
    )]);

    while let Some((module_name, scope, ancestry)) = queue.pop_front() {
        if !visited.insert((module_name.clone(), scope.clone())) {
            continue;
        }
        scopes
            .entry(module_name.clone())
            .or_default()
            .insert(scope.clone());
        let Some(module) = modules.get(&module_name) else {
            continue;
        };
        for instance in &module.instances {
            if modules.contains_key(&instance.module_name) {
                if ancestry.contains(&instance.module_name) {
                    warnings.push(format!(
                        "recursive module instantiation '{}' under '{}.{}' was not expanded",
                        instance.module_name, scope, instance.instance_name
                    ));
                    continue;
                }
                let suffix = instance
                    .generate_prefix
                    .iter()
                    .chain(std::iter::once(&instance.instance_name))
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(".");
                let mut child_ancestry = ancestry.clone();
                child_ancestry.push(instance.module_name.clone());
                queue.push_back((
                    instance.module_name.clone(),
                    format!("{scope}.{suffix}"),
                    child_ancestry,
                ));
            } else {
                let warning = format!(
                    "black-box module '{}' instantiated as '{}.{}'",
                    instance.module_name, scope, instance.instance_name
                );
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
    }
    scopes
}

fn backward_closure(
    roots: impl IntoIterator<Item = String>,
    dependencies: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let mut closure = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in roots {
        if closure.insert(root.clone()) {
            queue.push_back(root);
        }
    }
    while let Some(signal) = queue.pop_front() {
        if let Some(sources) = dependencies.get(&signal) {
            for source in sources {
                if closure.insert(source.clone()) {
                    queue.push_back(source.clone());
                }
            }
        }
    }
    closure
}

fn lhs_signals(tree: &SyntaxTree, lhs: RefNode<'_>) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for node in lhs.into_iter() {
        let name = match node {
            RefNode::HierarchicalIdentifier(identifier) => tree.get_str(identifier),
            RefNode::HierarchicalVariableIdentifier(identifier) => tree.get_str(identifier),
            RefNode::NetIdentifier(identifier) => tree.get_str(&identifier.nodes.0),
            RefNode::VariableIdentifier(identifier) => tree.get_str(&identifier.nodes.0),
            _ => None,
        };
        if let Some(name) = name.and_then(normalize_reference) {
            result.insert(name);
        }
    }
    result
}

fn identifiers_in_node(tree: &SyntaxTree, node: RefNode<'_>) -> BTreeSet<String> {
    tree.get_str(RefNodes(vec![node]))
        .map(lex_identifiers)
        .unwrap_or_default()
}

fn lex_identifiers(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut result = BTreeSet::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            let start = index;
            index += 1;
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index > start + 1 {
                result.insert(text[start..index].to_owned());
            }
            continue;
        }
        if is_identifier_start(bytes[index]) {
            let start = index;
            index += 1;
            while index < bytes.len() && is_identifier_continue(bytes[index]) {
                index += 1;
            }
            let prefix = text[..start].trim_end();
            if !prefix.ends_with('.') && !prefix.ends_with("::") {
                result.insert(text[start..index].to_owned());
            }
        } else {
            index += 1;
        }
    }
    result
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn normalize_reference(text: &str) -> Option<String> {
    lex_identifiers(text).into_iter().next()
}

fn retain_allowed(names: &mut BTreeSet<String>, allowed_names: &BTreeSet<String>) {
    names.retain(|name| allowed_names.contains(name));
}

fn module_name(tree: &SyntaxTree, root: RefNode<'_>) -> Option<String> {
    match root {
        RefNode::ModuleDeclarationAnsi(module) => {
            unwrap_node!(module, ModuleIdentifier).and_then(|node| match node {
                RefNode::ModuleIdentifier(identifier) => tree
                    .get_str(&identifier.nodes.0)
                    .map(|name| name.trim().to_owned()),
                _ => None,
            })
        }
        RefNode::ModuleDeclarationNonansi(module) => unwrap_node!(module, ModuleIdentifier)
            .and_then(|node| match node {
                RefNode::ModuleIdentifier(identifier) => tree
                    .get_str(&identifier.nodes.0)
                    .map(|name| name.trim().to_owned()),
                _ => None,
            }),
        RefNode::ModuleDeclarationWildcard(module) => tree
            .get_str(&module.nodes.3.nodes.0)
            .map(|name| name.trim().to_owned()),
        _ => None,
    }
}

fn node_text(tree: &SyntaxTree, node: RefNode<'_>) -> Option<String> {
    tree.get_str(RefNodes(vec![node]))
        .map(|text| text.trim().to_owned())
}

fn origin_line(tree: &SyntaxTree, source: &str, node: RefNode<'_>) -> Option<u32> {
    node.into_iter().find_map(|child| {
        let RefNode::Locate(locate) = child else {
            return None;
        };
        tree.get_origin(locate)
            .and_then(|(_, offset)| line_from_offset(source, offset))
    })
}

fn line_from_offset(source: &str, offset: usize) -> Option<u32> {
    if offset > source.len() {
        return None;
    }
    Some(
        (source[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1) as u32,
    )
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyze_sources(sources: &[(&str, &str)], top: &str) -> Report {
        let directory = tempfile::tempdir().unwrap();
        for (file, source) in sources {
            fs::write(directory.path().join(file), source).unwrap();
        }
        analyze_design(directory.path(), &[], top, "TOP.dut").unwrap()
    }

    fn excluded<'a>(report: &'a Report, module: &str) -> BTreeSet<&'a str> {
        report
            .modules
            .iter()
            .find(|entry| entry.module == module)
            .unwrap()
            .signals
            .iter()
            .map(|signal| signal.name.as_str())
            .collect()
    }

    #[test]
    fn excludes_inputs_and_dead_internal_signals() {
        let report = analyze_sources(
            &[(
                "top.sv",
                r#"module top(input logic a, input logic dead_i, output logic y);
  logic b;
  logic dead_b;
  assign b = a;
  assign y = b;
  assign dead_b = dead_i;
endmodule
"#,
            )],
            "top",
        );

        let names = excluded(&report, "top");
        assert!(names.contains("a"));
        assert!(names.contains("dead_i"));
        assert!(names.contains("dead_b"));
        assert!(!names.contains("b"));
        assert!(!names.contains("y"));
    }

    #[test]
    fn propagates_only_real_child_input_to_output_dependencies() {
        let report = analyze_sources(
            &[
                (
                    "leaf.sv",
                    r#"module leaf(input logic used_i, input logic dead_i, output logic y);
  assign y = used_i;
endmodule
"#,
                ),
                (
                    "top.sv",
                    r#"module top(input logic a, input logic d, output logic y);
  logic used_path;
  logic dead_path;
  logic child_y;
  assign used_path = a;
  assign dead_path = d;
  leaf u_leaf(.used_i(used_path), .dead_i(dead_path), .y(child_y));
  assign y = child_y;
endmodule
"#,
                ),
            ],
            "top",
        );

        let top_names = excluded(&report, "top");
        assert!(!top_names.contains("used_path"));
        assert!(!top_names.contains("child_y"));
        assert!(top_names.contains("dead_path"));
        assert_eq!(report.modules.len(), 2);
        assert_eq!(
            report
                .modules
                .iter()
                .find(|m| m.module == "leaf")
                .unwrap()
                .scopes,
            vec!["TOP.dut.u_leaf"]
        );
    }

    #[test]
    fn keeps_case_selector_as_a_control_dependency() {
        let report = analyze_sources(
            &[(
                "top.sv",
                r#"module top(input logic a, output logic y);
  logic select;
  logic value;
  assign select = a;
  always_comb begin
    case (1'b1)
      select: y = value;
      default: y = 1'b0;
    endcase
  end
endmodule
"#,
            )],
            "top",
        );

        let names = excluded(&report, "top");
        assert!(!names.contains("select"));
        assert!(!names.contains("value"));
    }

    #[test]
    fn keeps_both_ternary_operands() {
        let report = analyze_sources(
            &[(
                "top.sv",
                r#"module top(input logic sel, input logic src_left, input logic src_right, output logic y);
  logic left;
  logic right;
  logic selected;
  assign left = src_left;
  assign right = src_right;
  assign selected = sel ? left : right;
  assign y = selected;
endmodule
"#,
            )],
            "top",
        );

        let names = excluded(&report, "top");
        assert!(!names.contains("left"));
        assert!(!names.contains("right"));
        assert!(!names.contains("selected"));
        assert!(!names.contains("y"));
    }
}
