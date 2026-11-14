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
    // Warm up: the daemon is long-lived, and a cold CPU in powersave
    // otherwise measures frequency ramp-up rather than ranking.
    for _ in 0..2000 {
        std::hint::black_box(rank(&candidates, &q, &[], &HashMap::new()));
    }
    let mut times = vec![];
    for _ in 0..1001 {
        let start = Instant::now();
        let ranked = rank(&candidates, &q, &[], &HashMap::new());
        std::hint::black_box(ranked);
        times.push(start.elapsed().as_micros());
    }
    times.sort_unstable();
    eprintln!(
        "rank 1000: p50={}us p95={}us p99={}us",
        times[500], times[950], times[990]
    );
    assert!(times[950] < 1000, "ranking exceeds 1ms budget");
}
