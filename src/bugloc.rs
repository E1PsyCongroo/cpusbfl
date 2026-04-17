use crate::{coverage::*, fuzzer::CaseCoverage};

fn cal_suspicious(case_covs: &[CaseCoverage]) -> Vec<f64> {
    let len = cover_len();
    assert!(
        case_covs
            .iter()
            .all(|case_cov| case_cov.coverage.len() == len)
    );

    let mut e_p = vec![0usize; len];
    let mut e_f = vec![0usize; len];
    let mut n_p = vec![0usize; len];
    let mut n_f = vec![0usize; len];

    for case in case_covs {
        for (i, &covered) in case.coverage.iter().enumerate() {
            if case.is_passed {
                if covered != 0 {
                    e_p[i] += 1;
                } else {
                    n_p[i] += 1;
                }
            } else {
                if covered != 0 {
                    e_f[i] += 1;
                } else {
                    n_f[i] += 1;
                }
            }
        }
    }

    (0..len)
        .map(|i| {
            let ep = e_p[i] as f64;
            let ef = e_f[i] as f64;
            let nf = n_f[i] as f64;

            if ef == 0.0 {
                0.0
            } else {
                ef / ((ef + nf) * (ef + ep)).sqrt()
            }
        })
        .collect()
}

pub(crate) fn report_suspicious(case_covs: &[CaseCoverage], top_n: usize) -> () {
    let suspicious = cal_suspicious(case_covs);
    let mut indexed_suspicious: Vec<(usize, f64)> = suspicious.into_iter().enumerate().collect();
    indexed_suspicious.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    println!("Suspiciousness of cover points:");
    for (rank, (point, score)) in indexed_suspicious.iter().take(top_n).enumerate() {
        println!(
            "top-{}: Cover point {} with suspicious {:.6}",
            rank + 1,
            point,
            score
        );
    }
}
