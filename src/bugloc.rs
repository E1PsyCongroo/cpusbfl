use std::{
    collections::HashSet,
    fmt,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
};

use crate::block::{dfb::*, mgr::*, *};
use crate::coverage::*;
use crate::fuzzer::*;
use crate::spectrum::matrix::*;

pub(crate) fn report_result(
    top_sus: u64,
    metric: SpectrumMetric,
    case_metas: &[CaseMetadata],
    rtl_path: &Option<String>,
    include_paths: &Option<Vec<String>>,
    top_module: &Option<String>,
    top_scope: &Option<String>,
    output: &Option<String>,
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

    let mut ranked_points = cover_names()
        .into_iter()
        .flat_map(|cover_name| {
            calculate_suspiciousness(&cover_name, case_metas, metric)
                .into_iter()
                .enumerate()
                .map(move |(idx, sus)| (cover_name.clone(), idx, sus))
        })
        .collect::<Vec<(String, usize, f64)>>();
    ranked_points.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    let mut result_fs = output
        .as_ref()
        .map(|dirname| {
            let dirname = Path::new(dirname);
            File::create_new(dirname.join("result.log"))
        })
        .transpose()?;

    let initial_case = case_metas.iter().find(|case| !case.is_passed).unwrap();
    info_and_writeln(
        &mut result_fs,
        format_args!("Suspiciousness of cover points:"),
    )?;
    for (rank, (cover_name, point, score)) in
        ranked_points.iter().take(top_sus as usize).enumerate()
    {
        assert!(
            initial_case
                .covers
                .get(&cover_name)
                .covered_bits()
                .get(*point)
                .unwrap(),
            "point {} not in initial case",
            cover_point_name(&cover_name, *point),
        );

        info_and_writeln(
            &mut result_fs,
            format_args!(
                "top-{}: CoverPoint '{}' with suspicious '{:.6}'",
                rank + 1,
                cover_point_name(&cover_name, *point),
                score,
            ),
        )?;
    }

    if let Some((rtl_path, include_paths, top_module, top_scope)) = rtl_info {
        let mut ranked_blocks = sus_point2block(
            &ranked_points,
            &rtl_path,
            &include_paths,
            &top_module,
            &top_scope,
            output,
        )?;

        let mut seen = HashSet::new();

        ranked_blocks.retain(|(scope, bid, _score)| seen.insert((scope.clone(), *bid)));

        info_and_writeln(&mut result_fs, format_args!("Suspiciousness of block:"))?;

        for (rank, (scope, bid, score)) in ranked_blocks.iter().take(top_sus as usize).enumerate() {
            info_and_writeln(
                &mut result_fs,
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

fn info_and_writeln(output: &mut Option<File>, args: fmt::Arguments<'_>) -> io::Result<()> {
    log::info!("{args}");

    if let Some(file) = output.as_mut() {
        writeln!(file, "{args}")?;
    }

    Ok(())
}

fn get_module_files<P: AsRef<Path>>(path: P) -> Vec<PathBuf> {
    let path = path.as_ref();
    let mut files = Vec::new();

    // Check if the path is a file or directory
    if path.is_file() {
        // If it's a file, check if it has the appropriate extension
        if let Some(extension) = path.extension() {
            if extension == "sv" || extension == "v" {
                files.push(path.to_path_buf());
            }
        }
    } else if path.is_dir() {
        // If it's a directory, iterate through the entries
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.filter_map(Result::ok) {
                let entry_path = entry.path();
                // Only process files (skip subdirectories)
                if entry_path.is_file() {
                    // Check if the file has the appropriate extension
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
    rtl_path: &String,
    include_paths: &[String],
    top_module: &String,
    top_scope: &String,
    output: &Option<String>,
) -> Result<Vec<(String, u64, f64)>, Box<dyn std::error::Error>> {
    let rtl_files = get_module_files(&rtl_path);
    let includes = include_paths.iter().map(PathBuf::from).collect::<Vec<_>>();
    let block_mgr = BlockManager::new(&rtl_files, &includes, &top_module, &top_scope);
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

    for (cover_name, point, score) in sus_points.iter() {
        let point_name = cover_point_name(cover_name, *point);
        let (lineno, hier) = parse_cover_point_name(&point_name)?;
        match sbfl_blocks
            .iter()
            .find(|block| block.scope() == hier && block.line_ranges().contains(&lineno))
        {
            None => log::warn!("convert Point'{point_name}' to Block failed"),
            Some(block) => sus_blocks.push((block.scope().to_string(), block.bid(), *score)),
        }
    }

    Ok(sus_blocks)
}

fn parse_cover_point_name(name: &str) -> Result<(u32, &str), String> {
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

    let hier = name
        .split_once(", hier: ")
        .ok_or_else(|| format!("missing hier in cover point name: {name}"))?
        .1
        .trim();

    Ok((lineno, hier))
}
