use crate::{coverage::*, fuzzer::CaseMetadata};

fn cal_suspicious(cover_name: &String, case_meta: &[CaseMetadata]) -> Vec<f64> {
    let len = cover_len(cover_name);
    assert!(
        case_meta
            .iter()
            .all(|case_cov| case_cov.covers.get(cover_name).len() == len)
    );

    let mut e_p = vec![0usize; len];
    let mut e_f = vec![0usize; len];
    let mut n_p = vec![0usize; len];
    let mut n_f = vec![0usize; len];

    for case in case_meta {
        for (i, covered) in case
            .covers
            .get(cover_name)
            .covered_bits()
            .into_iter()
            .enumerate()
        {
            if case.is_passed {
                if covered {
                    e_p[i] += 1;
                } else {
                    n_p[i] += 1;
                }
            } else {
                if covered {
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

pub(crate) fn report_suspicious(case_meta: &[CaseMetadata], top_n: usize) -> () {
    let initial_case = case_meta.iter().find(|case| !case.is_passed).unwrap();
    let mut suspicious: Vec<(String, usize, f64)> = Vec::new();
    for cover_name in cover_names() {
        suspicious.extend(
            cal_suspicious(&cover_name, case_meta)
                .into_iter()
                .enumerate()
                .map(|(i, score)| (cover_name.to_owned(), i, score))
                .collect::<Vec<_>>(),
        );
    }
    suspicious.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    println!("Suspiciousness of cover points:");
    for (rank, (cover_name, point, score)) in suspicious.iter().take(top_n).enumerate() {
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
}
