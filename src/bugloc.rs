use std::{
    collections::HashSet,
    fmt,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::block::*;
use crate::coverage::*;
use crate::fuzzer::*;
use crate::spectrum::*;

struct PointBlockMapping {
    blocks: Vec<(String, u64, f64)>,
    toggle_assignments: Vec<ToggleAssignmentResult>,
}

struct ToggleAssignmentResult {
    point_name: String,
    scope: String,
    line: u32,
    statement: String,
    score: f64,
}

pub(crate) fn report_result(
    top_sus: u64,
    metric: SpectrumMetric,
    case_metas: Vec<CaseMetadata>,
    rtl_path: Option<impl AsRef<Path>>,
    include_paths: Option<&[PathBuf]>,
    top_module: Option<&str>,
    top_scope: Option<&str>,
    output: Option<impl AsRef<Path>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let rtl_info =
        match (rtl_path, include_paths, top_module, top_scope) {
            (Some(rtl_path), Some(include_paths), Some(top_module), Some(top_scope)) => {
                Some((rtl_path, include_paths, top_module, top_scope))
            }
            (None, None, None, None) => None,
            _ => return Err(
                "rtl_path, include_paths, top_module and top_scope must be all Some or all None"
                    .into(),
            ),
        };

    let initial_case = case_metas.iter().find(|case| !case.is_passed).unwrap();

    let mut ranked_points = cover_names()
        .into_iter()
        .flat_map(|cover_name| {
            calculate_suspiciousness(&cover_name, &case_metas, metric)
                .into_iter()
                .enumerate()
                .map(move |(idx, sus)| (cover_name.clone(), idx, sus))
        })
        .filter(|(_, _, sus)| *sus > 0.0)
        .collect::<Vec<(String, usize, f64)>>();
    ranked_points.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    let mut result_fs = output
        .as_ref()
        .map(|dirname| File::create_new(dirname.as_ref().join("result.log")))
        .transpose()?;

    info_and_writeln(
        result_fs.as_mut(),
        format_args!("Suspiciousness of cover points:"),
    )?;
    for (rank, (cover_name, point, score)) in
        ranked_points.iter().take(top_sus as usize).enumerate()
    {
        assert!(
            initial_case
                .covers
                .covered_bits(&cover_name)
                .get(*point)
                .unwrap(),
            "point {} not in initial case",
            cover_point_name(&cover_name, *point),
        );

        info_and_writeln(
            result_fs.as_mut(),
            format_args!(
                "top-{}: CoverPoint '{}' with suspicious '{:.6}'",
                rank + 1,
                cover_point_name(&cover_name, *point),
                score,
            ),
        )?;
    }

    if let Some((rtl_path, include_paths, top_module, top_scope)) = rtl_info {
        let mapping = sus_point2block(
            &ranked_points,
            &rtl_path,
            &include_paths,
            &top_module,
            &top_scope,
            output.as_ref(),
        )?;
        let mut ranked_blocks = mapping.blocks;

        let top_point_names = ranked_points
            .iter()
            .take(top_sus as usize)
            .map(|(cover_name, point, _)| cover_point_name(cover_name, *point))
            .collect::<HashSet<_>>();
        let mut seen_assignments = HashSet::new();
        let toggle_assignments = mapping
            .toggle_assignments
            .into_iter()
            .filter(|assignment| top_point_names.contains(&assignment.point_name))
            .filter(|assignment| {
                seen_assignments.insert((
                    assignment.point_name.clone(),
                    assignment.scope.clone(),
                    assignment.line,
                    assignment.statement.clone(),
                ))
            })
            .collect::<Vec<_>>();

        if !toggle_assignments.is_empty() {
            log::info!("Related assignment statements of toggle cover points:");
            for assignment in toggle_assignments {
                log::info!(
                    "Toggle CoverPoint '{}' -> Assignment(scope: {}, lineno: {}, statement: {:?}) with suspicious '{:.6}'",
                    assignment.point_name,
                    assignment.scope,
                    assignment.line,
                    assignment.statement,
                    assignment.score,
                );
            }
        }

        let mut seen = HashSet::new();

        ranked_blocks.retain(|(scope, bid, _score)| seen.insert((scope.clone(), *bid)));

        info_and_writeln(result_fs.as_mut(), format_args!("Suspiciousness of block:"))?;

        for (rank, (scope, bid, score)) in ranked_blocks.iter().take(top_sus as usize).enumerate() {
            info_and_writeln(
                result_fs.as_mut(),
                format_args!(
                    "top-{}: Block(scope: {}, bid: {}) with suspicious '{:.6}'",
                    rank + 1,
                    scope,
                    bid,
                    score,
                ),
            )?;
        }
    }

    Ok(())
}

fn info_and_writeln(output: Option<&mut File>, args: fmt::Arguments<'_>) -> io::Result<()> {
    log::info!("{args}");

    if let Some(file) = output {
        writeln!(file, "{args}")?;
    }

    Ok(())
}

fn get_module_files<P: AsRef<Path>>(path: P) -> Vec<PathBuf> {
    let path = path.as_ref();
    let mut files = Vec::new();

    if path.is_file() {
        if let Some(extension) = path.extension() {
            if extension == "sv" || extension == "v" {
                files.push(path.to_path_buf());
            }
        }
    } else if path.is_dir() {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.filter_map(Result::ok) {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    if let Some(extension) = entry_path.extension() {
                        if extension == "sv" || extension == "v" {
                            files.push(entry_path);
                        }
                    }
                }
            }
        }
    }

    files
}

fn sus_point2block(
    sus_points: &[(String, usize, f64)],
    rtl_path: impl AsRef<Path>,
    include_paths: &[PathBuf],
    top_module: &str,
    top_scope: &str,
    output: Option<impl AsRef<Path>>,
) -> Result<PointBlockMapping, Box<dyn std::error::Error>> {
    let rtl_files = get_module_files(&rtl_path);
    let block_mgr = BlockManager::new(&rtl_files, include_paths, &top_module, &top_scope);
    let all_blocks = block_mgr.get_all_blocks();
    let sbfl_blocks: Vec<_> = all_blocks
        .iter()
        .filter(|b| {
            !matches!(
                b.block_type(),
                BlockType::ModuleInput | BlockType::ModuleOutput
            )
        })
        .collect();

    if let Some(output) = output {
        block_mgr.dump_blocks_distribution(&output)?;
    }

    let mut sus_blocks: Vec<(String, u64, f64)> = Vec::new();
    let mut toggle_assignments = Vec::new();

    for (cover_name, point, score) in sus_points.iter() {
        let point_name = cover_point_name(cover_name, *point);
        let metadata = parse_cover_point_name(&point_name)?;

        if metadata.cover_type == "toggle" {
            let assignments = block_mgr.get_signal_assignments(metadata.hier, metadata.comment);
            if !assignments.is_empty() {
                let mut mapped_assignment = false;
                for assignment in assignments {
                    toggle_assignments.push(ToggleAssignmentResult {
                        point_name: point_name.clone(),
                        scope: metadata.hier.to_string(),
                        line: assignment.line(),
                        statement: assignment.statement().to_string(),
                        score: *score,
                    });

                    match sbfl_blocks.iter().find(|block| {
                        block.scope() == metadata.hier
                            && block.line_ranges().contains(&assignment.line())
                    }) {
                        None => log::warn!(
                            "convert toggle Point'{point_name}' assignment at line {} to Block failed",
                            assignment.line()
                        ),
                        Some(block) => {
                            mapped_assignment = true;
                            sus_blocks.push((block.scope().to_string(), block.bid(), *score))
                        }
                    }
                }
                if mapped_assignment {
                    continue;
                }
                log::warn!(
                    "no block contains assignments modifying toggle signal '{}' for Point'{point_name}', falling back to its coverage line",
                    metadata.comment
                );
            } else {
                log::warn!(
                    "no assignment modifying toggle signal '{}' found for Point'{point_name}', falling back to its coverage line",
                    metadata.comment
                );
            }
        }

        match sbfl_blocks.iter().find(|block| {
            block.scope() == metadata.hier && block.line_ranges().contains(&metadata.lineno)
        }) {
            None => log::warn!("convert Point'{point_name}' to Block failed"),
            Some(block) => sus_blocks.push((block.scope().to_string(), block.bid(), *score)),
        }
    }

    Ok(PointBlockMapping {
        blocks: sus_blocks,
        toggle_assignments,
    })
}

struct CoverPointMetadata<'a> {
    lineno: u32,
    cover_type: &'a str,
    comment: &'a str,
    hier: &'a str,
}

fn parse_cover_point_name(name: &str) -> Result<CoverPointMetadata<'_>, String> {
    let lineno = name
        .split_once("lineno: ")
        .ok_or_else(|| format!("missing lineno in cover point name: {name}"))?
        .1
        .split_once(", column: ")
        .ok_or_else(|| format!("missing column in cover point name: {name}"))?
        .0
        .trim()
        .parse::<u32>()
        .map_err(|err| format!("invalid lineno: {err}"))?;

    let cover_type = name
        .split_once(", type: ")
        .ok_or_else(|| format!("missing type in cover point name: {name}"))?
        .1
        .split_once(", linescov: ")
        .ok_or_else(|| format!("missing linescov in cover point name: {name}"))?
        .0
        .trim();

    let comment = name
        .split_once(", comment: ")
        .ok_or_else(|| format!("missing comment in cover point name: {name}"))?
        .1
        .split_once(", hier: ")
        .ok_or_else(|| format!("missing hier in cover point name: {name}"))?
        .0
        .trim();

    let hier = name
        .split_once(", hier: ")
        .ok_or_else(|| format!("missing hier in cover point name: {name}"))?
        .1
        .trim();

    Ok(CoverPointMetadata {
        lineno,
        cover_type,
        comment,
        hier,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_cover_point_name;

    #[test]
    fn parses_toggle_cover_point_metadata() {
        let point = "filename: rtl/top.sv, lineno: 42, column: 9, type: toggle, linescov: , comment: state_q[3], hier: TOP.top.u_dut";
        let metadata = parse_cover_point_name(point).unwrap();

        assert_eq!(metadata.lineno, 42);
        assert_eq!(metadata.cover_type, "toggle");
        assert_eq!(metadata.comment, "state_q[3]");
        assert_eq!(metadata.hier, "TOP.top.u_dut");
    }
}
