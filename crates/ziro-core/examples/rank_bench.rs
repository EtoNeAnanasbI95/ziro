use std::{collections::HashMap, time::Instant};
use ziro_core::{
    candidate::{Candidate, Kind},
    rank::{Query, rank},
};
fn main() {
    let candidates: Vec<_> = (0..1000)
        .map(|i| {
            let mut c = Candidate::history(format!("checkout-feature-{i:04}"), 0.);
            c.kind = Kind::Branch;
            c
        })
        .collect();
    let q = Query::parse("git checkout cf", 15);
    let mut times = vec![];
    for _ in 0..101 {
        let start = Instant::now();
        let ranked = rank(&candidates, &q, &[], &HashMap::new());
        std::hint::black_box(ranked);
        times.push(start.elapsed().as_micros());
    }
    times.sort_unstable();
    println!(
        "rank 1000: p50={}us p95={}us p99={}us",
        times[50], times[95], times[99]
    );
    assert!(times[95] < 1000, "ranking exceeds 1ms budget");
}
