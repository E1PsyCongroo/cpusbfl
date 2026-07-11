use clap::ValueEnum;
use serde::Serialize;

use crate::{coverage::*, fuzzer::CaseMetadata};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum, Serialize)]
#[value(rename_all = "lowercase")]
pub(crate) enum SpectrumMetric {
    Tarantula,
    Ochiai,
    Jaccard,
    Dstar,
    GP19,
    Barinel,
    Crosstab,
    Zoltar,
    Ample,
}

impl Default for SpectrumMetric {
    fn default() -> Self {
        SpectrumMetric::Ochiai
    }
}

#[derive(Debug, Clone, Copy)]
struct CoverageStats {
    ef: usize, // executed by failed tests
    ep: usize, // executed by passed tests
    nf: usize, // not executed by failed tests
    np: usize, // not executed by passed tests
}

pub(crate) fn calculate_suspiciousness(
    cover_name: &str,
    case_metas: &[CaseMetadata],
    metric: SpectrumMetric,
) -> Vec<f64> {
    calculate_coverage_stats(cover_name, case_metas)
        .into_iter()
        .map(|cover_stat| calculate_metric_score(cover_stat, metric))
        .collect()
}

fn calculate_coverage_stats(cover_name: &str, case_metas: &[CaseMetadata]) -> Vec<CoverageStats> {
    let len = cover_len(cover_name);
    assert!(
        case_metas
            .iter()
            .all(|case_cov| case_cov.covers.len(cover_name) == len)
    );

    let mut cover_stats = vec![
        CoverageStats {
            ef: 0,
            ep: 0,
            nf: 0,
            np: 0
        };
        len
    ];

    for CaseMetadata {
        covers,
        state_trackers: _,
        is_passed,
        mutated_pcs: _,
    } in case_metas
    {
        for (i, covered) in covers.covered_bits(cover_name).into_iter().enumerate() {
            match (covered, is_passed) {
                (true, false) => cover_stats[i].ef += 1,
                (true, true) => cover_stats[i].ep += 1,
                (false, false) => cover_stats[i].nf += 1,
                (false, true) => cover_stats[i].np += 1,
            }
        }
    }

    cover_stats
}

fn calculate_metric_score(stats: CoverageStats, metric: SpectrumMetric) -> f64 {
    let CoverageStats { ef, ep, nf, np } = stats;
    let ef = ef as f64;
    let ep = ep as f64;
    let nf = nf as f64;
    let np = np as f64;
    match metric {
        SpectrumMetric::Tarantula => {
            let fail_ratio = if ef + nf > 0.0 { ef / (ef + nf) } else { 0.0 };
            let pass_ratio = if ep + np > 0.0 { ep / (ep + np) } else { 0.0 };
            if fail_ratio + pass_ratio > 0.0 {
                fail_ratio / (fail_ratio + pass_ratio)
            } else {
                0.0
            }
        }
        SpectrumMetric::Ochiai => {
            let denom = ((ef + nf) * (ef + ep)).sqrt();
            if denom > 0.0 { ef / denom } else { 0.0 }
        }
        SpectrumMetric::Jaccard => {
            let denom = ef + nf + ep;
            if denom > 0.0 { ef / denom } else { 0.0 }
        }
        SpectrumMetric::Dstar => {
            let denom = ep + nf;
            if denom > 0.0 {
                ef.powi(2) / denom
            } else if ef > 0.0 {
                f64::INFINITY
            } else {
                0.0
            }
        }
        SpectrumMetric::GP19 => {
            if ef + nf > 0.0 {
                ef * (1.0 + 1.0 / (2.0 * ep + ef))
            } else {
                0.0
            }
        }
        SpectrumMetric::Barinel => {
            let h = ef + ep;
            let p = ef / (ef + nf).max(1.0);
            if h > 0.0 { 1.0 - p } else { 0.0 }
        }
        SpectrumMetric::Crosstab => {
            let n = ef + ep + nf + np;
            if n > 0.0 {
                let expected = (ef + ep) * (ef + nf) / n;
                if expected > 0.0 {
                    (ef - expected).abs() / expected.sqrt()
                } else {
                    0.0
                }
            } else {
                0.0
            }
        }
        SpectrumMetric::Zoltar => {
            if ef > 0.0 {
                let denom = ef + nf + ep + (10000.0 * nf * ep / ef);
                ef / denom
            } else {
                0.0
            }
        }
        SpectrumMetric::Ample => {
            let total_fail = ef + nf;
            let total_pass = ep + np;
            if total_fail > 0.0 && total_pass > 0.0 {
                (ef / total_fail - ep / total_pass).abs()
            } else if total_fail > 0.0 {
                ef / total_fail
            } else {
                0.0
            }
        }
    }
}
