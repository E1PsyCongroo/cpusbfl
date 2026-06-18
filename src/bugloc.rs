use std::path::{Path, PathBuf};

use crate::block::{dfb::*, mgr::*, *};
use crate::coverage::*;
use crate::fuzzer::*;
use crate::spectrum::matrix::*;

pub(crate) fn report_result(
    case_metas: &[CaseMetadata],
    top_sus: u64,
    rtl_dir: Option<String>,
    include_dir: Option<Vec<String>>,
    top_module: Option<String>,
    top_scope: Option<String>,
    metric: SpectrumMetric,
) -> Result<(), Box<dyn std::error::Error>> {
    let rtl_info = match (rtl_dir, include_dir, top_module, top_scope) {
        (Some(rtl_dir), Some(include_dir), Some(top_module), Some(top_scope)) => {
            Some((rtl_dir, include_dir, top_module, top_scope))
        }
        (None, None, None, None) => None,
        _ => panic!("rtl_dir, include_dir, top_module and top_scope must be all Some or all None"),
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

    if let Some((rtl_dir, include_dir, top_module, top_scope)) = rtl_info {
        let rtl_files = get_module_files(&rtl_dir);
        let includes = include_dir.iter().map(PathBuf::from).collect::<Vec<_>>();
        let block_mgr = BlockManager::new(&rtl_files, &includes, &top_module, &top_scope);
        block_mgr.dump_blocks_distribution("test")?;
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
    }

    let initial_case = case_metas.iter().find(|case| !case.is_passed).unwrap();
    println!("Suspiciousness of cover points:");
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
        println!(
            "top-{}: C '{}' with suspicious {:.6}",
            rank + 1,
            cover_point_name(&cover_name, *point),
            score
        );
    }

    Ok(())
}

fn get_module_files<P: AsRef<Path>>(path: P) -> Vec<std::path::PathBuf> {
    let path = path.as_ref();
    let mut files = Vec::new();
    if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.filter_map(Result::ok) {
                let p = entry.path();
                if p.is_file() {
                    if let Some(ext) = p.extension() {
                        if ext == "sv" || ext == "v" {
                            files.push(p);
                        }
                    }
                }
            }
        }
    }
    files
}
