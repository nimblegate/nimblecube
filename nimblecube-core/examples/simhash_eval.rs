//! SimHash encoder eval: does Hamming distance in the `Hv` actually track the
//! angle between the input vectors? Host/std, deterministic.
//!   cargo run --release --example simhash_eval
//!
//! Checks three things:
//!   1. the angle law `P[bit differs] = theta / pi`, against exact known angles
//!   2. the spread of "unrelated", which sets how confident a threshold can be
//!   3. recall@1 versus exact cosine on clustered data
//!
//! The projection uses +/-1 signs rather than Gaussian ones, so the angle law
//! is an approximation. Measuring how close it lands is the point of item 1.

use nimblecube_core::hv::DIM_BITS;
use nimblecube_core::simhash::simhash_f32;

const D: usize = 768; // a typical embedding width
const SEED: u64 = 0x5111_4A57_0000_0001;

fn xs(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

/// Uniform in [-1, 1).
fn rand_f32(s: &mut u64) -> f32 {
    ((xs(s) >> 40) as f32 / 8_388_608.0) - 1.0
}

fn rand_vec(s: &mut u64) -> Vec<f32> {
    (0..D).map(|_| rand_f32(s)).collect()
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum()
}

fn norm(a: &[f32]) -> f64 {
    dot(a, a).sqrt()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let d = norm(a) * norm(b);
    if d == 0.0 { 0.0 } else { dot(a, b) / d }
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = norm(&v);
    if n > 0.0 {
        for x in v.iter_mut() {
            *x = (*x as f64 / n) as f32;
        }
    }
    v
}

/// A pair at an exact angle: v = cos(t) * u + sin(t) * w, with w orthonormal to u.
fn pair_at_angle(t: f64, s: &mut u64) -> (Vec<f32>, Vec<f32>) {
    let u = unit(rand_vec(s));
    let mut w = rand_vec(s);
    let proj = dot(&w, &u);
    for i in 0..D {
        w[i] = (w[i] as f64 - proj * u[i] as f64) as f32;
    }
    let w = unit(w);
    let v = (0..D)
        .map(|i| (t.cos() * u[i] as f64 + t.sin() * w[i] as f64) as f32)
        .collect();
    (u, v)
}

fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn stddev(xs: &[f64]) -> f64 {
    let m = mean(xs);
    (xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / xs.len() as f64).sqrt()
}

fn angle_law(s: &mut u64) {
    println!("1. angle law: predicted 4096 * theta/pi vs measured Hamming");
    println!("   D={} trials=60 per row\n", D);
    println!("   {:>8}  {:>10}  {:>10}  {:>8}", "cosine", "predicted", "measured", "error");

    for target in [1.0_f64, 0.99, 0.95, 0.9, 0.8, 0.7, 0.5, 0.0, -0.5] {
        let t = target.clamp(-1.0, 1.0).acos();
        let predicted = DIM_BITS as f64 * t / std::f64::consts::PI;
        let measured: Vec<f64> = (0..60)
            .map(|_| {
                let (a, b) = pair_at_angle(t, s);
                simhash_f32(&a, SEED).hamming(&simhash_f32(&b, SEED)) as f64
            })
            .collect();
        let m = mean(&measured);
        println!(
            "   {:>8.2}  {:>10.0}  {:>10.0}  {:>+8.0}",
            target,
            predicted,
            m,
            m - predicted
        );
    }
}

fn unrelated_spread(s: &mut u64) {
    println!("\n2. spread of unrelated pairs (sets threshold confidence)");
    let n = 200;
    let d: Vec<f64> = (0..n)
        .map(|_| {
            let a = rand_vec(s);
            let b = rand_vec(s);
            simhash_f32(&a, SEED).hamming(&simhash_f32(&b, SEED)) as f64
        })
        .collect();
    let m = mean(&d);
    let sd = stddev(&d);

    // Two sources add in quadrature:
    //   bits:  4096 near-independent coin flips, sd = sqrt(DIM)/2
    //   input: two random D-dim vectors are not exactly orthogonal. Their cosine
    //          wobbles by about 1/sqrt(D), and near 90 degrees that carries into
    //          Hamming with slope DIM/pi.
    let sd_bits = (DIM_BITS as f64).sqrt() / 2.0;
    let sd_input = DIM_BITS as f64 / std::f64::consts::PI / (D as f64).sqrt();
    let sd_model = (sd_bits * sd_bits + sd_input * sd_input).sqrt();

    println!("   measured   mean={:.1} (se {:.1})  sd={:.1}", m, sd / (n as f64).sqrt(), sd);
    println!("   bits only  mean={:.1}            sd={:.1}", DIM_BITS as f64 / 2.0, sd_bits);
    println!("   with input wobble                sd={:.1}  (bits {:.1} + input {:.1})",
        sd_model, sd_bits, sd_input);
    println!("   so a hit at d=233 sits {:.0} sd below unrelated, not {:.0}",
        (m - 233.0) / sd, (DIM_BITS as f64 / 2.0 - 233.0) / sd_bits);
}

/// recall@1 alone conflates two things: whether the encoder preserves the
/// ordering, and whether the data has a distinguishable winner at all. With
/// items packed tightly around a base vector the true nearest can beat the
/// runner-up by a cosine hair no 4096-bit code could resolve. So this also
/// reports recall@10, whether the right cluster was found, and how much cosine
/// was actually given up by taking the SimHash pick over the true best.
fn recall_at_1(s: &mut u64) {
    println!("\n3. retrieval vs exact cosine, clustered data");
    let g = 100usize;
    let n = 600usize;
    let queries = 100usize;

    println!("   N={} groups={} queries={}\n", n, g, queries);
    println!(
        "   {:>7}  {:>9}  {:>9}  {:>9}  {:>12}",
        "jitter", "recall@1", "recall@10", "cluster", "cosine lost"
    );

    for jit in [0.15_f32, 0.45, 0.85] {
        let bases: Vec<Vec<f32>> = (0..g).map(|_| rand_vec(s)).collect();
        let jitter = |base: &[f32], s: &mut u64| -> Vec<f32> {
            base.iter().map(|x| x + jit * rand_f32(s)).collect()
        };
        let items: Vec<Vec<f32>> = (0..n).map(|i| jitter(&bases[i % g], s)).collect();
        let codes: Vec<_> = items.iter().map(|v| simhash_f32(v, SEED)).collect();

        let (mut top1, mut top10, mut cluster, mut lost) = (0usize, 0usize, 0usize, 0f64);
        for _ in 0..queries {
            let b = (xs(s) as usize) % g;
            let q = jitter(&bases[b], s);
            let qc = simhash_f32(&q, SEED);

            let cos: Vec<f64> = items.iter().map(|it| cosine(&q, it)).collect();
            let truth = (0..n).max_by(|&i, &j| cos[i].total_cmp(&cos[j])).unwrap();

            // One distance per item, not one per comparison in the sort.
            let dist: Vec<u32> = codes.iter().map(|c| qc.hamming(c)).collect();
            let mut order: Vec<usize> = (0..n).collect();
            order.sort_by_key(|&i| dist[i]);
            let got = order[0];

            if got == truth {
                top1 += 1;
            }
            if order[..10].contains(&truth) {
                top10 += 1;
            }
            if got % g == b {
                cluster += 1;
            }
            lost += cos[truth] - cos[got];
        }
        let pc = |x: usize| 100.0 * x as f64 / queries as f64;
        println!(
            "   {:>7.2}  {:>8.1}%  {:>8.1}%  {:>8.1}%  {:>12.4}",
            jit,
            pc(top1),
            pc(top10),
            pc(cluster),
            lost / queries as f64
        );
    }
}

fn main() {
    let mut s: u64 = 0x0051_11A5_4EED;
    println!("SimHash eval  DIM_BITS={}\n", DIM_BITS);
    angle_law(&mut s);
    unrelated_spread(&mut s);
    recall_at_1(&mut s);
}
