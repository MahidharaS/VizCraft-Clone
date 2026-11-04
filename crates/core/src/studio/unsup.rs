//! Unsupervised methods for the Studio: agglomerative clustering with the full merge history
//! (single / complete / average / Ward linkage, Lance–Williams updates) for dendrograms,
//! per-sample silhouette scores, classical multidimensional scaling (power iteration) and exact
//! t-SNE (perplexity calibration, early exaggeration, momentum gradient descent).

use crate::ml::{Matrix, Rng};

pub fn euclid(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>().sqrt()
}

pub fn distance_matrix(x: &Matrix) -> Matrix {
    let n = x.len();
    let mut d = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in i + 1..n {
            let v = euclid(&x[i], &x[j]);
            d[i][j] = v;
            d[j][i] = v;
        }
    }
    d
}

/// One merge: clusters `a` and `b` (ids < n are leaves, ≥ n are earlier merges) at `height`.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub a: usize,
    pub b: usize,
    pub height: f64,
    pub size: usize,
}

/// Agglomerative clustering; returns n − 1 merges (SciPy `linkage` order).
pub fn linkage(x: &Matrix, method: &str) -> Vec<Merge> {
    let n = x.len();
    if n < 2 {
        return vec![];
    }
    let mut d = distance_matrix(x);
    if method == "ward" {
        for r in d.iter_mut() {
            for v in r.iter_mut() {
                *v = *v * *v;
            }
        }
    }
    let mut active: Vec<bool> = vec![true; n];
    let mut id: Vec<usize> = (0..n).collect();
    let mut size: Vec<usize> = vec![1; n];
    let mut merges = Vec::with_capacity(n - 1);
    for step in 0..n - 1 {
        let mut best = (f64::INFINITY, 0, 0);
        for i in 0..n {
            if !active[i] {
                continue;
            }
            for j in i + 1..n {
                if active[j] && d[i][j] < best.0 {
                    best = (d[i][j], i, j);
                }
            }
        }
        let (h, i, j) = best;
        if !h.is_finite() {
            break;
        }
        let (si, sj) = (size[i] as f64, size[j] as f64);
        for k in 0..n {
            if !active[k] || k == i || k == j {
                continue;
            }
            let sk = size[k] as f64;
            let v = match method {
                "single" => d[i][k].min(d[j][k]),
                "complete" => d[i][k].max(d[j][k]),
                "ward" => ((si + sk) * d[i][k] + (sj + sk) * d[j][k] - sk * d[i][j]) / (si + sj + sk),
                _ => (si * d[i][k] + sj * d[j][k]) / (si + sj),
            };
            d[i][k] = v;
            d[k][i] = v;
        }
        active[j] = false;
        merges.push(Merge { a: id[i].min(id[j]), b: id[i].max(id[j]), height: if method == "ward" { h.max(0.0).sqrt() } else { h }, size: size[i] + size[j] });
        size[i] += size[j];
        id[i] = n + step;
    }
    merges
}

/// Cluster labels (0..k) from a linkage cut into `k` clusters.
pub fn cut(merges: &[Merge], n: usize, k: usize) -> Vec<usize> {
    let k = k.clamp(1, n.max(1));
    let mut parent: Vec<usize> = (0..n + merges.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    for (s, m) in merges.iter().enumerate().take(n.saturating_sub(k)) {
        let node = n + s;
        let (ra, rb) = (find(&mut parent, m.a), find(&mut parent, m.b));
        parent[ra] = node;
        parent[rb] = node;
    }
    let mut roots: Vec<usize> = vec![];
    let mut labels = vec![0; n];
    for (i, l) in labels.iter_mut().enumerate() {
        let r = find(&mut parent, i);
        let pos = match roots.iter().position(|x| *x == r) {
            Some(p) => p,
            None => {
                roots.push(r);
                roots.len() - 1
            }
        };
        *l = pos;
    }
    labels
}

/// Leaf order for drawing a dendrogram without crossings.
pub fn leaf_order(merges: &[Merge], n: usize) -> Vec<usize> {
    if n == 0 {
        return vec![];
    }
    if merges.is_empty() {
        return (0..n).collect();
    }
    let root = n + merges.len() - 1;
    let mut out = vec![];
    let mut stack = vec![root];
    while let Some(v) = stack.pop() {
        if v < n {
            out.push(v);
        } else if let Some(m) = merges.get(v - n) {
            stack.push(m.b);
            stack.push(m.a);
        }
    }
    out
}

/// Silhouette value of every sample (0 for singletons).
pub fn silhouettes(x: &Matrix, labels: &[usize]) -> Vec<f64> {
    let n = x.len().min(labels.len());
    let k = labels.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let mut out = vec![0.0; n];
    let sizes: Vec<usize> = (0..k).map(|c| labels.iter().take(n).filter(|l| **l == c).count()).collect();
    for i in 0..n {
        let mut sum = vec![0.0; k];
        for j in 0..n {
            if i != j {
                sum[labels[j]] += euclid(&x[i], &x[j]);
            }
        }
        let own = labels[i];
        if sizes[own] <= 1 {
            continue;
        }
        let a = sum[own] / (sizes[own] - 1) as f64;
        let b = (0..k).filter(|c| *c != own && sizes[*c] > 0).map(|c| sum[c] / sizes[c] as f64).fold(f64::INFINITY, f64::min);
        out[i] = if b.is_finite() { (b - a) / a.max(b).max(1e-300) } else { 0.0 };
    }
    out
}

/// Classical (Torgerson) MDS to `dims` dimensions.
pub fn mds(x: &Matrix, dims: usize, seed: u64) -> Matrix {
    let n = x.len();
    if n < 2 {
        return vec![vec![0.0; dims]; n];
    }
    let d = distance_matrix(x);
    // B = −½ J D² J
    let d2: Matrix = d.iter().map(|r| r.iter().map(|v| v * v).collect()).collect();
    let rm: Vec<f64> = d2.iter().map(|r| r.iter().sum::<f64>() / n as f64).collect();
    let gm = rm.iter().sum::<f64>() / n as f64;
    let b: Matrix = (0..n).map(|i| (0..n).map(|j| -0.5 * (d2[i][j] - rm[i] - rm[j] + gm)).collect()).collect();
    let mut rng = Rng::new(seed);
    let mut vecs: Vec<(f64, Vec<f64>)> = vec![];
    for _ in 0..dims {
        let mut v: Vec<f64> = (0..n).map(|_| rng.f64() - 0.5).collect();
        let mut lambda = 0.0;
        for _ in 0..300 {
            let mut w: Vec<f64> = b.iter().map(|r| r.iter().zip(&v).map(|(a, c)| a * c).sum()).collect();
            for (l, u) in &vecs {
                let p: f64 = u.iter().zip(&w).map(|(a, c)| a * c).sum();
                let _ = l;
                for (wi, ui) in w.iter_mut().zip(u) {
                    *wi -= p * ui;
                }
            }
            let norm = w.iter().map(|a| a * a).sum::<f64>().sqrt();
            if norm <= 1e-300 {
                break;
            }
            let nv: Vec<f64> = w.iter().map(|a| a / norm).collect();
            let diff: f64 = nv.iter().zip(&v).map(|(a, c)| (a - c).abs()).sum();
            v = nv;
            lambda = norm;
            if diff < 1e-10 {
                break;
            }
        }
        vecs.push((lambda, v));
    }
    (0..n).map(|i| vecs.iter().map(|(l, v)| v[i] * l.max(0.0).sqrt()).collect()).collect()
}

/// Exact t-SNE to 2-D.
pub fn tsne(x: &Matrix, perplexity: f64, iterations: usize, seed: u64) -> Matrix {
    let n = x.len();
    if n < 4 {
        return (0..n).map(|i| vec![i as f64, 0.0]).collect();
    }
    let perp = perplexity.clamp(2.0, ((n - 1) as f64 / 3.0).max(2.0));
    let d2: Matrix = (0..n).map(|i| (0..n).map(|j| x[i].iter().zip(&x[j]).map(|(a, b)| (a - b) * (a - b)).sum()).collect()).collect();
    // Conditional probabilities with binary search on the precision.
    let target = perp.ln();
    let mut p = vec![vec![0.0; n]; n];
    for i in 0..n {
        let (mut beta, mut lo, mut hi) = (1.0f64, 0.0f64, f64::INFINITY);
        let mind = (0..n).filter(|j| *j != i).map(|j| d2[i][j]).fold(f64::INFINITY, f64::min);
        for _ in 0..60 {
            let mut sum = 0.0;
            let mut hsum = 0.0;
            for j in 0..n {
                if j != i {
                    let v = (-(d2[i][j] - mind) * beta).exp();
                    p[i][j] = v;
                    sum += v;
                    hsum += v * (d2[i][j] - mind);
                }
            }
            let sum = sum.max(1e-300);
            let h = sum.ln() + beta * hsum / sum;
            for j in 0..n {
                p[i][j] /= sum;
            }
            if (h - target).abs() < 1e-5 {
                break;
            }
            if h > target {
                lo = beta;
                beta = if hi.is_finite() { (beta + hi) / 2.0 } else { beta * 2.0 };
            } else {
                hi = beta;
                beta = (beta + lo) / 2.0;
            }
        }
    }
    // Symmetrize.
    let mut pj = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..n {
            pj[i][j] = ((p[i][j] + p[j][i]) / (2.0 * n as f64)).max(1e-12);
        }
    }
    let mut rng = Rng::new(seed);
    let mut y: Matrix = (0..n).map(|_| vec![(rng.f64() - 0.5) * 1e-2, (rng.f64() - 0.5) * 1e-2]).collect();
    let mut vel = vec![vec![0.0; 2]; n];
    let mut gains = vec![vec![1.0; 2]; n];
    let iters = iterations.clamp(50, 2000);
    let eta = (n as f64 / 12.0).max(50.0);
    let mut q = vec![vec![0.0; n]; n];
    for it in 0..iters {
        let exag = if it < 100 { 12.0 } else { 1.0 };
        let mom = if it < 120 { 0.5 } else { 0.8 };
        let mut qsum = 0.0;
        for i in 0..n {
            for j in i + 1..n {
                let dx = y[i][0] - y[j][0];
                let dy = y[i][1] - y[j][1];
                let v = 1.0 / (1.0 + dx * dx + dy * dy);
                q[i][j] = v;
                q[j][i] = v;
                qsum += 2.0 * v;
            }
        }
        let qsum = qsum.max(1e-300);
        for i in 0..n {
            let mut g = [0.0, 0.0];
            for j in 0..n {
                if i == j {
                    continue;
                }
                let m = (exag * pj[i][j] - q[i][j] / qsum) * q[i][j];
                g[0] += 4.0 * m * (y[i][0] - y[j][0]);
                g[1] += 4.0 * m * (y[i][1] - y[j][1]);
            }
            for c in 0..2 {
                gains[i][c] = if (g[c] > 0.0) != (vel[i][c] > 0.0) { gains[i][c] + 0.2 } else { (gains[i][c] * 0.8f64).max(0.01) };
                vel[i][c] = mom * vel[i][c] - eta * gains[i][c] * g[c];
            }
        }
        for i in 0..n {
            y[i][0] += vel[i][0];
            y[i][1] += vel[i][1];
        }
        // Re-centre.
        let (mx, my) = (y.iter().map(|r| r[0]).sum::<f64>() / n as f64, y.iter().map(|r| r[1]).sum::<f64>() / n as f64);
        for r in y.iter_mut() {
            r[0] -= mx;
            r[1] -= my;
        }
    }
    y
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn blobs() -> Matrix {
        let mut rng = Rng::new(5);
        let mut x = vec![];
        for (cx, cy) in [(0.0, 0.0), (8.0, 8.0), (0.0, 8.0)] {
            for _ in 0..20 {
                x.push(vec![cx + rng.f64(), cy + rng.f64()]);
            }
        }
        x
    }

    #[test]
    fn linkage_reference() {
        // SciPy linkage([[0],[1],[5],[6],[20]], 'single') heights: 1, 1, 4, 14.
        let x: Matrix = [0.0, 1.0, 5.0, 6.0, 20.0].iter().map(|v| vec![*v]).collect();
        let m = linkage(&x, "single");
        let h: Vec<f64> = m.iter().map(|m| m.height).collect();
        assert_eq!(h, vec![1.0, 1.0, 4.0, 14.0]);
        let c = cut(&m, 5, 3);
        assert_eq!(c[0], c[1]);
        assert_eq!(c[2], c[3]);
        assert_ne!(c[0], c[4]);
        // Ward on two points: height = distance.
        let w = linkage(&vec![vec![0.0], vec![3.0]], "ward");
        assert!((w[0].height - 3.0).abs() < 1e-12);
        assert_eq!(leaf_order(&m, 5).len(), 5);
    }

    #[test]
    fn silhouette_and_embeddings() {
        let x = blobs();
        let labels: Vec<usize> = (0..60).map(|i| i / 20).collect();
        let s = silhouettes(&x, &labels);
        assert!(s.iter().sum::<f64>() / 60.0 > 0.8);
        let m = mds(&x, 2, 1);
        // MDS preserves distances for 2-D data.
        let d0 = euclid(&x[0], &x[30]);
        let d1 = euclid(&m[0], &m[30]);
        assert!((d0 - d1).abs() < 1e-6 * d0.max(1.0), "{d0} {d1}");
        let t = tsne(&x, 10.0, 300, 1);
        // Points of one blob stay closer than points of different blobs.
        let within = euclid(&t[0], &t[1]);
        let across = euclid(&t[0], &t[25]);
        assert!(within < across, "{within} {across}");
    }
}
