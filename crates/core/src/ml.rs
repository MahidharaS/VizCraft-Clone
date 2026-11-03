//! Machine learning and statistics (Orange/KNIME-style widgets), implemented from scratch with
//! standard textbook algorithms: k-means (k-means++ init, silhouette for automatic k),
//! hierarchical clustering (average linkage), DBSCAN, PCA (Jacobi eigen decomposition), linear
//! regression (OLS), logistic regression (gradient descent, one-vs-rest), CART decision trees,
//! random forests (bagging + feature subsampling), Gaussian naive Bayes, k-nearest neighbours,
//! evaluation metrics, outlier detection, correlations, feature statistics and histograms.
//!
//! Everything is deterministic: random choices use [`Rng`] seeded from the node settings.

use std::collections::BTreeMap;

use crate::build::{ColType, TableData};
use crate::scalar::Scalar;

// =============================================================================================
// Random numbers (SplitMix64)
// =============================================================================================

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in 0..n.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() % n as u64) as usize }
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

// =============================================================================================
// Matrices from tables
// =============================================================================================

pub type Matrix = Vec<Vec<f64>>;

/// Numeric columns of a table (types Number/Integer, or Any with numeric values).
pub fn numeric_columns(t: &TableData) -> Vec<String> {
    t.columns
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let ty = t.types.get(*i).copied().unwrap_or_default();
            ty.is_numeric() || (ty == ColType::Any && t.rows.iter().take(200).any(|r| matches!(r.get(*i), Some(Scalar::Num(_)))))
        })
        .map(|(_, c)| c.clone())
        .collect()
}

/// The values of `columns` as a matrix; missing / non-numeric values are NaN.
pub fn matrix(t: &TableData, columns: &[String]) -> Result<Matrix, String> {
    let idx: Vec<usize> = columns.iter().map(|c| t.column_index(c).ok_or_else(|| format!("There's no column `{c}`."))).collect::<Result<_, _>>()?;
    Ok(t.rows.iter().map(|r| idx.iter().map(|i| r.get(*i).and_then(Scalar::as_f64).unwrap_or(f64::NAN)).collect()).collect())
}

/// Replaces NaN with the column mean; returns the means.
pub fn impute_mean(x: &mut Matrix) -> Vec<f64> {
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let mut means = vec![0.0; d];
    for (j, m) in means.iter_mut().enumerate() {
        let vals: Vec<f64> = x.iter().filter_map(|r| r.get(j).copied()).filter(|v| v.is_finite()).collect();
        *m = if vals.is_empty() { 0.0 } else { vals.iter().sum::<f64>() / vals.len() as f64 };
    }
    for r in x.iter_mut() {
        for (j, v) in r.iter_mut().enumerate() {
            if !v.is_finite() {
                *v = means.get(j).copied().unwrap_or(0.0);
            }
        }
    }
    means
}

/// Standardisation parameters (mean, standard deviation per feature).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scaler {
    pub mean: Vec<f64>,
    pub std: Vec<f64>,
}

impl Scaler {
    pub fn fit(x: &Matrix) -> Scaler {
        let d = x.first().map(|r| r.len()).unwrap_or(0);
        let n = x.len().max(1) as f64;
        let mut mean = vec![0.0; d];
        for r in x {
            for (j, v) in r.iter().enumerate() {
                if let Some(m) = mean.get_mut(j) {
                    *m += v / n;
                }
            }
        }
        let mut std = vec![0.0; d];
        for r in x {
            for (j, v) in r.iter().enumerate() {
                if let Some(s) = std.get_mut(j) {
                    *s += (v - mean.get(j).copied().unwrap_or(0.0)).powi(2) / n;
                }
            }
        }
        let std = std.into_iter().map(|v| if v.sqrt() > 1e-12 { v.sqrt() } else { 1.0 }).collect();
        Scaler { mean, std }
    }
    pub fn apply_row(&self, r: &[f64]) -> Vec<f64> {
        r.iter().enumerate().map(|(j, v)| (v - self.mean.get(j).copied().unwrap_or(0.0)) / self.std.get(j).copied().unwrap_or(1.0)).collect()
    }
    pub fn apply(&self, x: &Matrix) -> Matrix {
        x.iter().map(|r| self.apply_row(r)).collect()
    }
}

fn dist2(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

// =============================================================================================
// Clustering
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct KMeans {
    pub labels: Vec<usize>,
    pub centroids: Matrix,
    pub inertia: f64,
    pub iterations: usize,
}

/// k-means with k-means++ initialisation (Lloyd iterations).
pub fn kmeans(x: &Matrix, k: usize, seed: u64, max_iter: usize) -> KMeans {
    let n = x.len();
    let k = k.clamp(1, n.max(1));
    if n == 0 {
        return KMeans::default();
    }
    let mut rng = Rng::new(seed);
    let mut centroids: Matrix = vec![x.get(rng.below(n)).cloned().unwrap_or_default()];
    let mut d2: Vec<f64> = x.iter().map(|p| dist2(p, centroids.first().map(Vec::as_slice).unwrap_or(&[]))).collect();
    while centroids.len() < k {
        let total: f64 = d2.iter().sum();
        let next = if total <= 0.0 {
            rng.below(n)
        } else {
            let mut target = rng.f64() * total;
            let mut pick = n - 1;
            for (i, d) in d2.iter().enumerate() {
                target -= d;
                if target <= 0.0 {
                    pick = i;
                    break;
                }
            }
            pick
        };
        let c = x.get(next).cloned().unwrap_or_default();
        for (i, p) in x.iter().enumerate() {
            let dd = dist2(p, &c);
            if let Some(slot) = d2.get_mut(i)
                && dd < *slot
            {
                *slot = dd;
            }
        }
        centroids.push(c);
    }
    let dim = x.first().map(|r| r.len()).unwrap_or(0);
    let mut labels = vec![0usize; n];
    let mut iterations = 0;
    for it in 0..max_iter.max(1) {
        iterations = it + 1;
        let mut changed = false;
        for (i, p) in x.iter().enumerate() {
            let best = nearest(p, &centroids);
            if labels.get(i) != Some(&best) || it == 0 {
                if labels.get(i) != Some(&best) {
                    changed = true;
                }
                if let Some(l) = labels.get_mut(i) {
                    *l = best;
                }
            }
        }
        let mut sums = vec![vec![0.0; dim]; k];
        let mut counts = vec![0usize; k];
        for (p, l) in x.iter().zip(&labels) {
            if let (Some(s), Some(c)) = (sums.get_mut(*l), counts.get_mut(*l)) {
                for (a, b) in s.iter_mut().zip(p) {
                    *a += b;
                }
                *c += 1;
            }
        }
        for (j, c) in centroids.iter_mut().enumerate() {
            let cnt = counts.get(j).copied().unwrap_or(0);
            if cnt > 0
                && let Some(s) = sums.get(j)
            {
                *c = s.iter().map(|v| v / cnt as f64).collect();
            }
        }
        if !changed && it > 0 {
            break;
        }
    }
    let inertia = x.iter().zip(&labels).map(|(p, l)| centroids.get(*l).map(|c| dist2(p, c)).unwrap_or(0.0)).sum();
    KMeans { labels, centroids, inertia, iterations }
}

fn nearest(p: &[f64], centroids: &Matrix) -> usize {
    let mut best = 0;
    let mut bd = f64::INFINITY;
    for (j, c) in centroids.iter().enumerate() {
        let d = dist2(p, c);
        if d < bd {
            bd = d;
            best = j;
        }
    }
    best
}

/// Mean silhouette coefficient (on up to 1500 points, deterministic subsample).
pub fn silhouette(x: &Matrix, labels: &[usize]) -> f64 {
    let n = x.len();
    let k = labels.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    if n < 3 || k < 2 {
        return 0.0;
    }
    let step = (n / 1500).max(1);
    let idx: Vec<usize> = (0..n).step_by(step).collect();
    let mut total = 0.0;
    let mut cnt = 0;
    for &i in &idx {
        let li = labels.get(i).copied().unwrap_or(0);
        let mut sums = vec![0.0; k];
        let mut counts = vec![0usize; k];
        for &j in &idx {
            if i == j {
                continue;
            }
            let lj = labels.get(j).copied().unwrap_or(0);
            if let (Some(s), Some(c)) = (sums.get_mut(lj), counts.get_mut(lj)) {
                *s += dist2(x.get(i).map(Vec::as_slice).unwrap_or(&[]), x.get(j).map(Vec::as_slice).unwrap_or(&[])).sqrt();
                *c += 1;
            }
        }
        let own = counts.get(li).copied().unwrap_or(0);
        if own == 0 {
            continue;
        }
        let a = sums.get(li).copied().unwrap_or(0.0) / own as f64;
        let b = (0..k).filter(|c| *c != li && counts.get(*c).copied().unwrap_or(0) > 0).map(|c| sums.get(c).copied().unwrap_or(0.0) / counts.get(c).copied().unwrap_or(1) as f64).fold(f64::INFINITY, f64::min);
        if b.is_finite() {
            total += (b - a) / a.max(b).max(1e-12);
            cnt += 1;
        }
    }
    if cnt == 0 { 0.0 } else { total / cnt as f64 }
}

/// Tries k = 2..=max_k and keeps the k with the best silhouette. Returns (result, k, scores).
pub fn kmeans_auto(x: &Matrix, max_k: usize, seed: u64) -> (KMeans, usize, Vec<(usize, f64)>) {
    let mut best: Option<(KMeans, usize, f64)> = None;
    let mut scores = Vec::new();
    let max_k = max_k.min(x.len().saturating_sub(1)).max(2);
    for k in 2..=max_k {
        let r = kmeans(x, k, seed, 100);
        let s = silhouette(x, &r.labels);
        scores.push((k, s));
        if best.as_ref().is_none_or(|b| s > b.2 + 1e-12) {
            best = Some((r, k, s));
        }
    }
    match best {
        Some((r, k, _)) => (r, k, scores),
        None => (kmeans(x, 1, seed, 10), 1, scores),
    }
}

/// Agglomerative clustering with average linkage, cut into `k` clusters. Labels are numbered by
/// first appearance.
pub fn hierarchical(x: &Matrix, k: usize) -> Result<Vec<usize>, String> {
    let n = x.len();
    if n > 3000 {
        return Err(format!("Hierarchical clustering handles up to 3,000 rows ({n} given): add a Sample node first."));
    }
    if n == 0 {
        return Ok(vec![]);
    }
    let k = k.clamp(1, n);
    // Distance matrix (condensed into a full matrix of f32 to save memory).
    let mut d = vec![vec![0f32; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let v = dist2(x.get(i).map(Vec::as_slice).unwrap_or(&[]), x.get(j).map(Vec::as_slice).unwrap_or(&[])).sqrt() as f32;
            if let Some(r) = d.get_mut(i) {
                r[j] = v;
            }
            if let Some(r) = d.get_mut(j) {
                r[i] = v;
            }
        }
    }
    let mut active: Vec<bool> = vec![true; n];
    let mut size: Vec<usize> = vec![1; n];
    let mut members: Vec<Vec<usize>> = (0..n).map(|i| vec![i]).collect();
    // Nearest neighbour per active cluster.
    let nn = |d: &Vec<Vec<f32>>, active: &Vec<bool>, i: usize| -> (usize, f32) {
        let mut best = (usize::MAX, f32::INFINITY);
        if let Some(row) = d.get(i) {
            for (j, v) in row.iter().enumerate() {
                if j != i && active.get(j).copied().unwrap_or(false) && *v < best.1 {
                    best = (j, *v);
                }
            }
        }
        best
    };
    let mut nearest: Vec<(usize, f32)> = (0..n).map(|i| nn(&d, &active, i)).collect();
    let mut clusters = n;
    while clusters > k {
        // Closest pair.
        let mut a = usize::MAX;
        let mut best = f32::INFINITY;
        for i in 0..n {
            if active.get(i).copied().unwrap_or(false)
                && let Some((_, v)) = nearest.get(i)
                && *v < best
            {
                best = *v;
                a = i;
            }
        }
        if a == usize::MAX {
            break;
        }
        let b = nearest.get(a).map(|x| x.0).unwrap_or(usize::MAX);
        if b == usize::MAX {
            break;
        }
        // Merge b into a (Lance–Williams update for average linkage).
        let (sa, sb) = (size.get(a).copied().unwrap_or(1) as f32, size.get(b).copied().unwrap_or(1) as f32);
        for j in 0..n {
            if j == a || j == b || !active.get(j).copied().unwrap_or(false) {
                continue;
            }
            let da = d.get(a).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            let db = d.get(b).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
            let v = (sa * da + sb * db) / (sa + sb);
            if let Some(r) = d.get_mut(a) {
                r[j] = v;
            }
            if let Some(r) = d.get_mut(j) {
                r[a] = v;
            }
        }
        if let Some(s) = active.get_mut(b) {
            *s = false;
        }
        if let Some(s) = size.get_mut(a) {
            *s += sb as usize;
        }
        let mb = std::mem::take(members.get_mut(b).unwrap_or(&mut Vec::new()));
        if let Some(m) = members.get_mut(a) {
            m.extend(mb);
        }
        clusters -= 1;
        // Refresh nearest neighbours that pointed at a or b.
        for i in 0..n {
            if !active.get(i).copied().unwrap_or(false) {
                continue;
            }
            let cur = nearest.get(i).map(|x| x.0).unwrap_or(usize::MAX);
            if i == a || cur == a || cur == b {
                let v = nn(&d, &active, i);
                if let Some(s) = nearest.get_mut(i) {
                    *s = v;
                }
            } else {
                let da = d.get(i).and_then(|r| r.get(a)).copied().unwrap_or(f32::INFINITY);
                if let Some(s) = nearest.get_mut(i)
                    && da < s.1
                {
                    *s = (a, da);
                }
            }
        }
    }
    let mut labels = vec![0usize; n];
    let mut order: Vec<usize> = (0..n).filter(|i| active.get(*i).copied().unwrap_or(false)).collect();
    order.sort_by_key(|c| members.get(*c).and_then(|m| m.iter().min().copied()).unwrap_or(0));
    for (lab, c) in order.iter().enumerate() {
        for &m in members.get(*c).map(Vec::as_slice).unwrap_or(&[]) {
            if let Some(l) = labels.get_mut(m) {
                *l = lab;
            }
        }
    }
    Ok(labels)
}

/// DBSCAN: returns a label per point; `None` is noise.
pub fn dbscan(x: &Matrix, eps: f64, min_pts: usize) -> Vec<Option<usize>> {
    let n = x.len();
    let eps2 = eps * eps;
    let neighbours = |i: usize| -> Vec<usize> {
        let p = x.get(i).map(Vec::as_slice).unwrap_or(&[]);
        (0..n).filter(|j| dist2(p, x.get(*j).map(Vec::as_slice).unwrap_or(&[])) <= eps2).collect()
    };
    let mut labels: Vec<Option<usize>> = vec![None; n];
    let mut visited = vec![false; n];
    let mut cluster = 0;
    for i in 0..n {
        if visited.get(i).copied().unwrap_or(true) {
            continue;
        }
        if let Some(v) = visited.get_mut(i) {
            *v = true;
        }
        let nb = neighbours(i);
        if nb.len() < min_pts {
            continue;
        }
        if let Some(l) = labels.get_mut(i) {
            *l = Some(cluster);
        }
        let mut queue = nb;
        let mut qi = 0;
        while qi < queue.len() {
            let j = queue.get(qi).copied().unwrap_or(0);
            qi += 1;
            if !visited.get(j).copied().unwrap_or(true) {
                if let Some(v) = visited.get_mut(j) {
                    *v = true;
                }
                let nb2 = neighbours(j);
                if nb2.len() >= min_pts {
                    queue.extend(nb2);
                }
            }
            if let Some(l) = labels.get_mut(j)
                && l.is_none()
            {
                *l = Some(cluster);
            }
        }
        cluster += 1;
    }
    labels
}

// =============================================================================================
// PCA
// =============================================================================================

/// Eigen decomposition of a symmetric matrix by cyclic Jacobi rotations. Returns eigenvalues
/// (descending) and eigenvectors (as columns: `vecs[i][k]` is component k's weight of feature i).
pub fn jacobi_eigen(a: &Matrix) -> (Vec<f64>, Matrix) {
    let n = a.len();
    let mut m = a.clone();
    let mut v: Matrix = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    let get = |m: &Matrix, i: usize, j: usize| m.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
    let set = |m: &mut Matrix, i: usize, j: usize, x: f64| {
        if let Some(r) = m.get_mut(i)
            && let Some(c) = r.get_mut(j)
        {
            *c = x;
        }
    };
    for _sweep in 0..100 {
        let off: f64 = (0..n).flat_map(|i| (0..n).filter(move |j| *j != i).map(move |j| (i, j))).map(|(i, j)| get(&m, i, j).powi(2)).sum();
        if off < 1e-22 {
            break;
        }
        for p in 0..n {
            for q in (p + 1)..n {
                let apq = get(&m, p, q);
                if apq.abs() < 1e-300 {
                    continue;
                }
                let app = get(&m, p, p);
                let aqq = get(&m, q, q);
                let theta = (aqq - app) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let t = if theta == 0.0 { 1.0 } else { t };
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for k in 0..n {
                    let mkp = get(&m, k, p);
                    let mkq = get(&m, k, q);
                    set(&mut m, k, p, c * mkp - s * mkq);
                    set(&mut m, k, q, s * mkp + c * mkq);
                }
                for k in 0..n {
                    let mpk = get(&m, p, k);
                    let mqk = get(&m, q, k);
                    set(&mut m, p, k, c * mpk - s * mqk);
                    set(&mut m, q, k, s * mpk + c * mqk);
                }
                for k in 0..n {
                    let vkp = get(&v, k, p);
                    let vkq = get(&v, k, q);
                    set(&mut v, k, p, c * vkp - s * vkq);
                    set(&mut v, k, q, s * vkp + c * vkq);
                }
            }
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| get(&m, *b, *b).partial_cmp(&get(&m, *a, *a)).unwrap_or(std::cmp::Ordering::Equal));
    let vals: Vec<f64> = order.iter().map(|i| get(&m, *i, *i)).collect();
    let mut vecs: Matrix = vec![vec![0.0; n]; n];
    for (k, &i) in order.iter().enumerate() {
        // Sign convention: the largest weight is positive.
        let col: Vec<f64> = (0..n).map(|r| get(&v, r, i)).collect();
        let big = col.iter().copied().fold(0.0f64, |a, b| if b.abs() > a.abs() { b } else { a });
        let sign = if big < 0.0 { -1.0 } else { 1.0 };
        for (r, x) in col.iter().enumerate() {
            set(&mut vecs, r, k, x * sign);
        }
    }
    (vals, vecs)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pca {
    /// Eigenvalues (variance per component).
    pub variances: Vec<f64>,
    pub explained: Vec<f64>,
    /// `loadings[feature][component]`.
    pub loadings: Matrix,
    /// Projected rows (`scores[row][component]`).
    pub scores: Matrix,
}

/// PCA on (optionally standardised) data, keeping `k` components.
pub fn pca(x: &Matrix, k: usize, standardize: bool) -> Pca {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    if n < 2 || d == 0 {
        return Pca::default();
    }
    let sc = Scaler::fit(x);
    let z: Matrix = if standardize { sc.apply(x) } else { x.iter().map(|r| r.iter().enumerate().map(|(j, v)| v - sc.mean.get(j).copied().unwrap_or(0.0)).collect()).collect() };
    let mut cov = vec![vec![0.0; d]; d];
    for r in &z {
        for i in 0..d {
            for j in i..d {
                let v = r.get(i).copied().unwrap_or(0.0) * r.get(j).copied().unwrap_or(0.0) / (n - 1) as f64;
                if let Some(row) = cov.get_mut(i)
                    && let Some(c) = row.get_mut(j)
                {
                    *c += v;
                }
            }
        }
    }
    for i in 0..d {
        for j in 0..i {
            let v = cov.get(j).and_then(|r| r.get(i)).copied().unwrap_or(0.0);
            if let Some(row) = cov.get_mut(i)
                && let Some(c) = row.get_mut(j)
            {
                *c = v;
            }
        }
    }
    let (vals, vecs) = jacobi_eigen(&cov);
    let total: f64 = vals.iter().map(|v| v.max(0.0)).sum();
    let k = k.clamp(1, d);
    let scores = z.iter().map(|r| (0..k).map(|c| r.iter().enumerate().map(|(i, v)| v * vecs.get(i).and_then(|row| row.get(c)).copied().unwrap_or(0.0)).sum()).collect()).collect();
    Pca {
        explained: vals.iter().take(k).map(|v| if total > 0.0 { v.max(0.0) / total } else { 0.0 }).collect(),
        variances: vals.iter().take(k).copied().collect(),
        loadings: vecs.iter().map(|r| r.iter().take(k).copied().collect()).collect(),
        scores,
    }
}

// =============================================================================================
// Linear algebra helpers
// =============================================================================================

/// Inverts a square matrix (Gauss–Jordan with partial pivoting).
pub fn invert(a: &Matrix) -> Option<Matrix> {
    let n = a.len();
    let mut m: Matrix = a.iter().enumerate().map(|(i, r)| r.iter().copied().chain((0..n).map(|j| if i == j { 1.0 } else { 0.0 })).collect()).collect();
    for col in 0..n {
        let piv = (col..n).max_by(|x, y| {
            let a = m.get(*x).and_then(|r| r.get(col)).copied().unwrap_or(0.0).abs();
            let b = m.get(*y).and_then(|r| r.get(col)).copied().unwrap_or(0.0).abs();
            a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
        })?;
        let pv = m.get(piv).and_then(|r| r.get(col)).copied().unwrap_or(0.0);
        if pv.abs() < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        let prow: Vec<f64> = m.get(col).map(|r| r.iter().map(|v| v / pv).collect()).unwrap_or_default();
        if let Some(r) = m.get_mut(col) {
            *r = prow.clone();
        }
        for (i, row) in m.iter_mut().enumerate() {
            if i == col {
                continue;
            }
            let f = row.get(col).copied().unwrap_or(0.0);
            if f != 0.0 {
                for (x, p) in row.iter_mut().zip(&prow) {
                    *x -= f * p;
                }
            }
        }
    }
    Some(m.into_iter().map(|r| r.into_iter().skip(n).collect()).collect())
}

// =============================================================================================
// Supervised learning
// =============================================================================================

/// A decision tree node (CART).
#[derive(Clone, Debug, PartialEq)]
pub enum TreeNode {
    Leaf { value: Vec<f64>, n: usize },
    Split { feature: usize, threshold: f64, left: usize, right: usize, n: usize },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub nodes: Vec<TreeNode>,
    pub depth: usize,
}

impl Tree {
    fn predict(&self, r: &[f64]) -> &[f64] {
        let mut i = 0;
        for _ in 0..10_000 {
            match self.nodes.get(i) {
                Some(TreeNode::Leaf { value, .. }) => return value,
                Some(TreeNode::Split { feature, threshold, left, right, .. }) => {
                    i = if r.get(*feature).copied().unwrap_or(0.0) <= *threshold { *left } else { *right };
                }
                None => break,
            }
        }
        &[]
    }
}

#[derive(Clone, Debug)]
pub struct TreeParams {
    pub max_depth: usize,
    pub min_samples_split: usize,
    pub min_samples_leaf: usize,
    /// Features tried per split (None = all).
    pub max_features: Option<usize>,
}

impl Default for TreeParams {
    fn default() -> Self {
        TreeParams { max_depth: 6, min_samples_split: 4, min_samples_leaf: 2, max_features: None }
    }
}

/// Targets: class indices (classification) or values (regression).
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Class(Vec<usize>, usize),
    Value(Vec<f64>),
}

impl Target {
    fn len(&self) -> usize {
        match self {
            Target::Class(c, _) => c.len(),
            Target::Value(v) => v.len(),
        }
    }
}

fn leaf_value(y: &Target, idx: &[usize]) -> Vec<f64> {
    match y {
        Target::Class(c, k) => {
            let mut counts = vec![0.0; *k];
            for &i in idx {
                if let Some(s) = c.get(i).and_then(|cl| counts.get_mut(*cl)) {
                    *s += 1.0;
                }
            }
            let n = idx.len().max(1) as f64;
            counts.iter().map(|v| v / n).collect()
        }
        Target::Value(v) => {
            let n = idx.len().max(1) as f64;
            vec![idx.iter().map(|i| v.get(*i).copied().unwrap_or(0.0)).sum::<f64>() / n]
        }
    }
}

pub fn fit_tree(x: &Matrix, y: &Target, p: &TreeParams, rng: &mut Rng) -> Tree {
    let idx: Vec<usize> = (0..y.len().min(x.len())).collect();
    let mut tree = Tree::default();
    grow(x, y, p, rng, &mut tree, idx, 0);
    tree
}

fn grow(x: &Matrix, y: &Target, p: &TreeParams, rng: &mut Rng, tree: &mut Tree, idx: Vec<usize>, depth: usize) -> usize {
    let me = tree.nodes.len();
    tree.depth = tree.depth.max(depth);
    tree.nodes.push(TreeNode::Leaf { value: leaf_value(y, &idx), n: idx.len() });
    if depth >= p.max_depth || idx.len() < p.min_samples_split {
        return me;
    }
    let Some((feature, threshold)) = best_split(x, y, &idx, p, rng) else { return me };
    let (l, r): (Vec<usize>, Vec<usize>) = idx.iter().partition(|i| x.get(**i).and_then(|row| row.get(feature)).copied().unwrap_or(0.0) <= threshold);
    if l.is_empty() || r.is_empty() {
        return me;
    }
    let n = idx.len();
    let left = grow(x, y, p, rng, tree, l, depth + 1);
    let right = grow(x, y, p, rng, tree, r, depth + 1);
    if let Some(node) = tree.nodes.get_mut(me) {
        *node = TreeNode::Split { feature, threshold, left, right, n };
    }
    me
}

fn best_split(x: &Matrix, y: &Target, idx: &[usize], p: &TreeParams, rng: &mut Rng) -> Option<(usize, f64)> {
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let mut feats: Vec<usize> = (0..d).collect();
    if let Some(m) = p.max_features
        && m < d
    {
        rng.shuffle(&mut feats);
        feats.truncate(m.max(1));
    }
    let n = idx.len();
    let mut best: Option<(usize, f64, f64)> = None;
    let val = |i: usize, f: usize| x.get(i).and_then(|r| r.get(f)).copied().unwrap_or(0.0);
    for &f in &feats {
        let mut order: Vec<usize> = idx.to_vec();
        order.sort_by(|a, b| val(*a, f).partial_cmp(&val(*b, f)).unwrap_or(std::cmp::Ordering::Equal));
        match y {
            Target::Class(c, k) => {
                let mut left = vec![0.0; *k];
                let mut right = vec![0.0; *k];
                for &i in &order {
                    if let Some(s) = c.get(i).and_then(|cl| right.get_mut(*cl)) {
                        *s += 1.0;
                    }
                }
                let gini = |counts: &[f64], tot: f64| if tot <= 0.0 { 0.0 } else { 1.0 - counts.iter().map(|v| (v / tot).powi(2)).sum::<f64>() };
                for pos in 0..n.saturating_sub(1) {
                    let i = order.get(pos).copied().unwrap_or(0);
                    if let Some(cl) = c.get(i) {
                        if let Some(s) = left.get_mut(*cl) {
                            *s += 1.0;
                        }
                        if let Some(s) = right.get_mut(*cl) {
                            *s -= 1.0;
                        }
                    }
                    let nl = pos + 1;
                    let nr = n - nl;
                    if nl < p.min_samples_leaf || nr < p.min_samples_leaf {
                        continue;
                    }
                    let (a, b) = (val(i, f), val(order.get(pos + 1).copied().unwrap_or(0), f));
                    if a == b {
                        continue;
                    }
                    let score = (nl as f64 * gini(&left, nl as f64) + nr as f64 * gini(&right, nr as f64)) / n as f64;
                    if best.as_ref().is_none_or(|bb| score < bb.2 - 1e-12) {
                        best = Some((f, (a + b) / 2.0, score));
                    }
                }
            }
            Target::Value(v) => {
                let yv = |i: usize| v.get(i).copied().unwrap_or(0.0);
                let (mut sl, mut ql) = (0.0, 0.0);
                let st: f64 = order.iter().map(|i| yv(*i)).sum();
                let qt: f64 = order.iter().map(|i| yv(*i).powi(2)).sum();
                for pos in 0..n.saturating_sub(1) {
                    let i = order.get(pos).copied().unwrap_or(0);
                    sl += yv(i);
                    ql += yv(i).powi(2);
                    let nl = (pos + 1) as f64;
                    let nr = n as f64 - nl;
                    if (pos + 1) < p.min_samples_leaf || (n - pos - 1) < p.min_samples_leaf {
                        continue;
                    }
                    let (a, b) = (val(i, f), val(order.get(pos + 1).copied().unwrap_or(0), f));
                    if a == b {
                        continue;
                    }
                    let sse_l = ql - sl * sl / nl;
                    let sse_r = (qt - ql) - (st - sl).powi(2) / nr;
                    let score = sse_l + sse_r;
                    if best.as_ref().is_none_or(|bb| score < bb.2 - 1e-9) {
                        best = Some((f, (a + b) / 2.0, score));
                    }
                }
            }
        }
    }
    best.map(|(f, t, _)| (f, t))
}

/// The algorithm behind a trained model.
#[derive(Clone, Debug, PartialEq)]
pub enum Learner {
    Linear { coef: Vec<f64>, std_err: Vec<f64> },
    Logistic { weights: Vec<Vec<f64>> },
    Tree(Tree),
    Forest(Vec<Tree>),
    NaiveBayes { prior: Vec<f64>, mean: Matrix, var: Matrix },
    Knn { x: Matrix, y: Target, k: usize },
}

/// A trained model: features, target, classes, preprocessing and the learner.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub kind: String,
    pub features: Vec<String>,
    pub target: String,
    /// Class labels (classification); empty for regression.
    pub classes: Vec<String>,
    pub fill: Vec<f64>,
    pub scaler: Option<Scaler>,
    pub learner: Learner,
    /// Training metrics and notes.
    pub info: Vec<(String, String)>,
}

impl Model {
    pub fn is_classifier(&self) -> bool {
        !self.classes.is_empty()
    }
    /// Predicts one row of raw feature values: (value or class index, class probabilities).
    pub fn predict_row(&self, raw: &[f64]) -> (f64, Vec<f64>) {
        let filled: Vec<f64> = raw.iter().enumerate().map(|(j, v)| if v.is_finite() { *v } else { self.fill.get(j).copied().unwrap_or(0.0) }).collect();
        let r = match &self.scaler {
            Some(s) => s.apply_row(&filled),
            None => filled,
        };
        let k = self.classes.len();
        let probs: Vec<f64> = match &self.learner {
            Learner::Linear { coef, .. } => {
                let v = coef.first().copied().unwrap_or(0.0) + r.iter().zip(coef.iter().skip(1)).map(|(a, b)| a * b).sum::<f64>();
                return (v, vec![]);
            }
            Learner::Logistic { weights } => {
                let raw: Vec<f64> = weights.iter().map(|w| sigmoid(w.first().copied().unwrap_or(0.0) + r.iter().zip(w.iter().skip(1)).map(|(a, b)| a * b).sum::<f64>())).collect();
                if k == 2 {
                    let p1 = raw.first().copied().unwrap_or(0.5);
                    vec![1.0 - p1, p1]
                } else {
                    let s: f64 = raw.iter().sum::<f64>().max(1e-12);
                    raw.iter().map(|v| v / s).collect()
                }
            }
            Learner::Tree(t) => {
                let v = t.predict(&r);
                if k == 0 {
                    return (v.first().copied().unwrap_or(0.0), vec![]);
                }
                v.to_vec()
            }
            Learner::Forest(ts) => {
                if k == 0 {
                    let s: f64 = ts.iter().map(|t| t.predict(&r).first().copied().unwrap_or(0.0)).sum();
                    return (s / ts.len().max(1) as f64, vec![]);
                }
                let mut acc = vec![0.0; k];
                for t in ts {
                    for (a, b) in acc.iter_mut().zip(t.predict(&r)) {
                        *a += b;
                    }
                }
                acc.iter().map(|v| v / ts.len().max(1) as f64).collect()
            }
            Learner::NaiveBayes { prior, mean, var } => {
                let logs: Vec<f64> = (0..k)
                    .map(|c| {
                        let mut lp = prior.get(c).copied().unwrap_or(1e-9).max(1e-12).ln();
                        for (j, x) in r.iter().enumerate() {
                            let m = mean.get(c).and_then(|v| v.get(j)).copied().unwrap_or(0.0);
                            let s2 = var.get(c).and_then(|v| v.get(j)).copied().unwrap_or(1.0).max(1e-9);
                            lp += -0.5 * ((x - m).powi(2) / s2 + (2.0 * std::f64::consts::PI * s2).ln());
                        }
                        lp
                    })
                    .collect();
                let mx = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let e: Vec<f64> = logs.iter().map(|v| (v - mx).exp()).collect();
                let s: f64 = e.iter().sum::<f64>().max(1e-300);
                e.iter().map(|v| v / s).collect()
            }
            Learner::Knn { x, y, k: kk } => {
                let mut d: Vec<(f64, usize)> = x.iter().enumerate().map(|(i, p)| (dist2(p, &r), i)).collect();
                let kk = (*kk).clamp(1, d.len().max(1));
                if kk < d.len() {
                    d.select_nth_unstable_by(kk - 1, |a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                }
                let near = d.get(..kk).unwrap_or(&[]);
                match y {
                    Target::Value(v) => {
                        let s: f64 = near.iter().map(|(_, i)| v.get(*i).copied().unwrap_or(0.0)).sum();
                        return (s / near.len().max(1) as f64, vec![]);
                    }
                    Target::Class(c, _) => {
                        let mut acc = vec![0.0; k];
                        for (_, i) in near {
                            if let Some(s) = c.get(*i).and_then(|cl| acc.get_mut(*cl)) {
                                *s += 1.0;
                            }
                        }
                        acc.iter().map(|v| v / near.len().max(1) as f64).collect()
                    }
                }
            }
        };
        let best = probs.iter().enumerate().fold((0usize, f64::NEG_INFINITY), |acc, (i, p)| if *p > acc.1 { (i, *p) } else { acc }).0;
        (best as f64, probs)
    }
}

fn sigmoid(z: f64) -> f64 {
    1.0 / (1.0 + (-z.clamp(-40.0, 40.0)).exp())
}

/// Settings for [`train`].
#[derive(Clone, Debug)]
pub struct TrainParams {
    /// "linear", "logistic", "tree", "forest", "naive_bayes", "knn".
    pub kind: String,
    /// "auto", "classification" or "regression".
    pub task: String,
    pub tree: TreeParams,
    pub trees: usize,
    pub k: usize,
    pub iterations: usize,
    pub learning_rate: f64,
    pub l2: f64,
    pub seed: u64,
}

impl Default for TrainParams {
    fn default() -> Self {
        TrainParams { kind: "tree".into(), task: "auto".into(), tree: TreeParams::default(), trees: 50, k: 5, iterations: 400, learning_rate: 0.3, l2: 1e-3, seed: 42 }
    }
}

/// Is a target column categorical? Text / boolean → yes; numbers → only when few integer values.
pub fn is_categorical(t: &TableData, col: usize) -> bool {
    let ty = t.types.get(col).copied().unwrap_or_default();
    if matches!(ty, ColType::Text | ColType::Bool) {
        return true;
    }
    let mut distinct = std::collections::BTreeSet::new();
    for r in &t.rows {
        match r.get(col) {
            Some(Scalar::Num(n)) => {
                if n.fract() != 0.0 {
                    return false;
                }
                distinct.insert(*n as i64);
                if distinct.len() > 10 {
                    return false;
                }
            }
            Some(Scalar::Text(_)) | Some(Scalar::Bool(_)) => return true,
            _ => {}
        }
    }
    true
}

/// Trains a model on `features` → `target` of a table.
pub fn train(t: &TableData, features: &[String], target: &str, p: &TrainParams) -> Result<Model, String> {
    let ti = t.column_index(target).ok_or_else(|| format!("There's no target column `{target}`."))?;
    if features.is_empty() {
        return Err("Choose at least one feature column.".into());
    }
    // Rows with a target value.
    let keep: Vec<usize> = (0..t.rows.len()).filter(|r| !t.cell(*r, ti).is_null()).collect();
    if keep.len() < 2 {
        return Err("Not enough rows with a target value to learn from.".into());
    }
    let full = matrix(t, features)?;
    let mut x: Matrix = keep.iter().filter_map(|r| full.get(*r).cloned()).collect();
    let fill = impute_mean(&mut x);
    let classify = match p.task.as_str() {
        "classification" => true,
        "regression" => false,
        _ => p.kind == "logistic" || p.kind == "naive_bayes" || is_categorical(t, ti),
    };
    if !classify && (p.kind == "logistic" || p.kind == "naive_bayes") {
        return Err("This learner predicts categories: pick a categorical target (or use Linear regression).".into());
    }
    if classify && p.kind == "linear" {
        return Err("Linear regression predicts numbers: pick a numeric target (or use Logistic regression).".into());
    }
    let (y, classes) = if classify {
        let labels: Vec<String> = keep.iter().map(|r| t.cell(*r, ti).text()).collect();
        let mut classes: Vec<String> = labels.clone();
        classes.sort_by(|a, b| match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
            _ => a.cmp(b),
        });
        classes.dedup();
        if classes.len() < 2 {
            return Err("The target has a single class: nothing to learn.".into());
        }
        if classes.len() > 50 {
            return Err(format!("The target has {} classes: too many for classification.", classes.len()));
        }
        let ids = labels.iter().map(|l| classes.iter().position(|c| c == l).unwrap_or(0)).collect();
        (Target::Class(ids, classes.len()), classes)
    } else {
        (Target::Value(keep.iter().map(|r| t.cell(*r, ti).as_f64().unwrap_or(0.0)).collect()), vec![])
    };
    let scale = matches!(p.kind.as_str(), "logistic" | "knn");
    let scaler = if scale { Some(Scaler::fit(&x)) } else { None };
    let xs = match &scaler {
        Some(s) => s.apply(&x),
        None => x.clone(),
    };
    let mut rng = Rng::new(p.seed);
    let mut info = Vec::new();
    let learner = match p.kind.as_str() {
        "linear" => {
            let Target::Value(yv) = &y else { return Err("Linear regression needs a numeric target.".into()) };
            let (coef, se, r2, adj) = ols(&xs, yv)?;
            info.push(("R²".into(), format!("{r2:.4}")));
            info.push(("Adjusted R²".into(), format!("{adj:.4}")));
            Learner::Linear { coef, std_err: se }
        }
        "logistic" => {
            let Target::Class(c, k) = &y else { return Err("Logistic regression needs a categorical target.".into()) };
            let models = if *k == 2 { 1 } else { *k };
            let weights = (0..models)
                .map(|m| {
                    let cls = if *k == 2 { 1 } else { m };
                    let yb: Vec<f64> = c.iter().map(|v| if *v == cls { 1.0 } else { 0.0 }).collect();
                    logistic_gd(&xs, &yb, p.iterations, p.learning_rate, p.l2)
                })
                .collect();
            Learner::Logistic { weights }
        }
        "tree" => {
            let tree = fit_tree(&xs, &y, &p.tree, &mut rng);
            info.push(("Depth".into(), tree.depth.to_string()));
            info.push(("Leaves".into(), tree.nodes.iter().filter(|n| matches!(n, TreeNode::Leaf { .. })).count().to_string()));
            Learner::Tree(tree)
        }
        "forest" => {
            let d = features.len();
            let mf = if classify { (d as f64).sqrt().ceil() as usize } else { (d / 3).max(1) };
            let tp = TreeParams { max_features: Some(mf.max(1)), ..p.tree.clone() };
            let n = xs.len();
            let trees: Vec<Tree> = (0..p.trees.clamp(1, 500))
                .map(|_| {
                    let idx: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
                    let bx: Matrix = idx.iter().filter_map(|i| xs.get(*i).cloned()).collect();
                    let by = match &y {
                        Target::Class(c, k) => Target::Class(idx.iter().map(|i| c.get(*i).copied().unwrap_or(0)).collect(), *k),
                        Target::Value(v) => Target::Value(idx.iter().map(|i| v.get(*i).copied().unwrap_or(0.0)).collect()),
                    };
                    fit_tree(&bx, &by, &tp, &mut rng)
                })
                .collect();
            info.push(("Trees".into(), trees.len().to_string()));
            info.push(("Features per split".into(), mf.to_string()));
            Learner::Forest(trees)
        }
        "naive_bayes" => {
            let Target::Class(c, k) = &y else { return Err("Naive Bayes needs a categorical target.".into()) };
            let d = features.len();
            let mut prior: Vec<f64> = vec![0.0; *k];
            let mut mean = vec![vec![0.0; d]; *k];
            let mut var = vec![vec![0.0; d]; *k];
            for (r, cl) in xs.iter().zip(c) {
                if let Some(pp) = prior.get_mut(*cl) {
                    *pp += 1.0;
                }
                if let Some(m) = mean.get_mut(*cl) {
                    for (a, b) in m.iter_mut().zip(r) {
                        *a += b;
                    }
                }
            }
            for (cl, m) in mean.iter_mut().enumerate() {
                let cnt = prior.get(cl).copied().unwrap_or(0.0).max(1.0);
                for a in m.iter_mut() {
                    *a /= cnt;
                }
            }
            for (r, cl) in xs.iter().zip(c) {
                if let (Some(v), Some(m)) = (var.get_mut(*cl), mean.get(*cl)) {
                    for ((a, b), mm) in v.iter_mut().zip(r).zip(m) {
                        *a += (b - mm).powi(2);
                    }
                }
            }
            // Variance smoothing relative to the largest feature variance.
            let maxvar = Scaler::fit(&xs).std.iter().map(|s| s * s).fold(0.0, f64::max);
            for (cl, v) in var.iter_mut().enumerate() {
                let cnt = prior.get(cl).copied().unwrap_or(0.0).max(1.0);
                for a in v.iter_mut() {
                    *a = *a / cnt + 1e-9 * maxvar.max(1.0);
                }
            }
            let n = xs.len() as f64;
            for pp in prior.iter_mut() {
                *pp /= n;
            }
            Learner::NaiveBayes { prior, mean, var }
        }
        "knn" => Learner::Knn { x: xs.clone(), y: y.clone(), k: p.k.max(1) },
        other => return Err(format!("Unknown learner `{other}`.")),
    };
    let mut model = Model { kind: p.kind.clone(), features: features.to_vec(), target: target.to_string(), classes, fill, scaler, learner, info: vec![] };
    // Training fit.
    let preds: Vec<(f64, Vec<f64>)> = x.iter().map(|r| model.predict_row(r)).collect();
    match &y {
        Target::Class(c, _) => {
            let acc = c.iter().zip(&preds).filter(|(a, b)| **a == b.0 as usize).count() as f64 / c.len().max(1) as f64;
            info.insert(0, ("Training accuracy".into(), format!("{:.1}%", acc * 100.0)));
        }
        Target::Value(v) => {
            if p.kind != "linear" {
                let r = regression_metrics(v, &preds.iter().map(|p| p.0).collect::<Vec<_>>());
                info.insert(0, ("Training R²".into(), format!("{:.4}", r.r2)));
            }
        }
    }
    info.push(("Training rows".into(), x.len().to_string()));
    model.info = info;
    Ok(model)
}

/// Ordinary least squares with an intercept. Returns (coefficients [intercept first], standard
/// errors, R², adjusted R²).
pub fn ols(x: &Matrix, y: &[f64]) -> Result<(Vec<f64>, Vec<f64>, f64, f64), String> {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0) + 1;
    if n < d {
        return Err(format!("Linear regression needs more rows ({n}) than coefficients ({d})."));
    }
    let row = |i: usize| std::iter::once(1.0).chain(x.get(i).cloned().unwrap_or_default());
    let mut xtx = vec![vec![0.0; d]; d];
    let mut xty = vec![0.0; d];
    for i in 0..n {
        let r: Vec<f64> = row(i).collect();
        let yi = y.get(i).copied().unwrap_or(0.0);
        for a in 0..d {
            let ra = r.get(a).copied().unwrap_or(0.0);
            if let Some(s) = xty.get_mut(a) {
                *s += ra * yi;
            }
            if let Some(rowm) = xtx.get_mut(a) {
                for (b, rb) in r.iter().enumerate() {
                    if let Some(s) = rowm.get_mut(b) {
                        *s += ra * rb;
                    }
                }
            }
        }
    }
    for (a, rowm) in xtx.iter_mut().enumerate().skip(1) {
        if let Some(s) = rowm.get_mut(a) {
            *s += 1e-10;
        }
    }
    let inv = invert(&xtx).ok_or("The features are collinear (one is a combination of others): remove some.")?;
    let coef: Vec<f64> = inv.iter().map(|r| r.iter().zip(&xty).map(|(a, b)| a * b).sum()).collect();
    let mean = y.iter().sum::<f64>() / n.max(1) as f64;
    let mut sse = 0.0;
    let mut sst = 0.0;
    for i in 0..n {
        let pred: f64 = row(i).zip(&coef).map(|(a, b)| a * b).sum();
        let yi = y.get(i).copied().unwrap_or(0.0);
        sse += (yi - pred).powi(2);
        sst += (yi - mean).powi(2);
    }
    let r2 = if sst > 0.0 { 1.0 - sse / sst } else { 0.0 };
    let dof = (n as f64 - d as f64).max(1.0);
    let adj = 1.0 - (1.0 - r2) * (n as f64 - 1.0) / dof;
    let sigma2 = sse / dof;
    let se = (0..d).map(|i| (sigma2 * inv.get(i).and_then(|r| r.get(i)).copied().unwrap_or(0.0)).max(0.0).sqrt()).collect();
    Ok((coef, se, r2, adj))
}

fn logistic_gd(x: &Matrix, y: &[f64], iters: usize, lr: f64, l2: f64) -> Vec<f64> {
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    let n = x.len().max(1) as f64;
    let mut w = vec![0.0; d + 1];
    for _ in 0..iters.max(1) {
        let mut g = vec![0.0; d + 1];
        for (r, yi) in x.iter().zip(y) {
            let z = w.first().copied().unwrap_or(0.0) + r.iter().zip(w.iter().skip(1)).map(|(a, b)| a * b).sum::<f64>();
            let e = sigmoid(z) - yi;
            if let Some(g0) = g.first_mut() {
                *g0 += e;
            }
            for (gj, xj) in g.iter_mut().skip(1).zip(r) {
                *gj += e * xj;
            }
        }
        for (j, (wj, gj)) in w.iter_mut().zip(&g).enumerate() {
            let reg = if j == 0 { 0.0 } else { l2 * *wj };
            *wj -= lr * (gj / n + reg);
        }
    }
    w
}

// =============================================================================================
// Metrics
// =============================================================================================

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RegressionMetrics {
    pub rmse: f64,
    pub mae: f64,
    pub r2: f64,
    pub n: usize,
}

pub fn regression_metrics(actual: &[f64], pred: &[f64]) -> RegressionMetrics {
    let n = actual.len().min(pred.len());
    if n == 0 {
        return RegressionMetrics::default();
    }
    let mean = actual.iter().take(n).sum::<f64>() / n as f64;
    let (mut sse, mut sae, mut sst) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (a, p) = (actual.get(i).copied().unwrap_or(0.0), pred.get(i).copied().unwrap_or(0.0));
        sse += (a - p).powi(2);
        sae += (a - p).abs();
        sst += (a - mean).powi(2);
    }
    RegressionMetrics { rmse: (sse / n as f64).sqrt(), mae: sae / n as f64, r2: if sst > 0.0 { 1.0 - sse / sst } else { 0.0 }, n }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassMetrics {
    pub accuracy: f64,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
    /// Area under the ROC curve (macro one-vs-rest); None without probabilities.
    pub auc: Option<f64>,
    /// `confusion[actual][predicted]`.
    pub confusion: Vec<Vec<usize>>,
    pub n: usize,
}

/// Classification metrics; `probs[i][class]` are optional scores for AUC.
pub fn class_metrics(actual: &[usize], pred: &[usize], k: usize, probs: Option<&[Vec<f64>]>) -> ClassMetrics {
    let n = actual.len().min(pred.len());
    let mut cm = vec![vec![0usize; k]; k];
    for i in 0..n {
        let (a, p) = (actual.get(i).copied().unwrap_or(0), pred.get(i).copied().unwrap_or(0));
        if let Some(c) = cm.get_mut(a).and_then(|r| r.get_mut(p)) {
            *c += 1;
        }
    }
    let correct: usize = (0..k).map(|c| cm.get(c).and_then(|r| r.get(c)).copied().unwrap_or(0)).sum();
    let present: Vec<usize> = (0..k).filter(|c| cm.get(*c).map(|r| r.iter().sum::<usize>()).unwrap_or(0) > 0 || (0..k).any(|a| cm.get(a).and_then(|r| r.get(*c)).copied().unwrap_or(0) > 0)).collect();
    let (mut ps, mut rs, mut fs) = (0.0, 0.0, 0.0);
    for &c in &present {
        let tp = cm.get(c).and_then(|r| r.get(c)).copied().unwrap_or(0) as f64;
        let pred_c: f64 = (0..k).map(|a| cm.get(a).and_then(|r| r.get(c)).copied().unwrap_or(0) as f64).sum();
        let act_c: f64 = cm.get(c).map(|r| r.iter().sum::<usize>() as f64).unwrap_or(0.0);
        let p = if pred_c > 0.0 { tp / pred_c } else { 0.0 };
        let r = if act_c > 0.0 { tp / act_c } else { 0.0 };
        ps += p;
        rs += r;
        fs += if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 };
    }
    let m = present.len().max(1) as f64;
    // Binary problems report precision / recall / F1 of the positive (second) class.
    let (precision, recall, f1) = if k == 2 {
        let tp = cm.get(1).and_then(|r| r.get(1)).copied().unwrap_or(0) as f64;
        let fp = cm.first().and_then(|r| r.get(1)).copied().unwrap_or(0) as f64;
        let fneg = cm.get(1).and_then(|r| r.first()).copied().unwrap_or(0) as f64;
        let p = if tp + fp > 0.0 { tp / (tp + fp) } else { 0.0 };
        let r = if tp + fneg > 0.0 { tp / (tp + fneg) } else { 0.0 };
        (p, r, if p + r > 0.0 { 2.0 * p * r / (p + r) } else { 0.0 })
    } else {
        (ps / m, rs / m, fs / m)
    };
    let auc = probs.map(|pr| {
        let classes: Vec<usize> = if k == 2 { vec![1] } else { (0..k).collect() };
        let mut total = 0.0;
        let mut cnt = 0;
        for c in classes {
            let scores: Vec<(f64, bool)> = (0..n).map(|i| (pr.get(i).and_then(|r| r.get(c)).copied().unwrap_or(0.0), actual.get(i) == Some(&c))).collect();
            if let Some(a) = auc_binary(&scores) {
                total += a;
                cnt += 1;
            }
        }
        if cnt == 0 { 0.5 } else { total / cnt as f64 }
    });
    ClassMetrics { accuracy: if n > 0 { correct as f64 / n as f64 } else { 0.0 }, precision, recall, f1, auc, confusion: cm, n }
}

/// ROC AUC via the rank-sum (Mann–Whitney) statistic with tie handling.
pub fn auc_binary(scores: &[(f64, bool)]) -> Option<f64> {
    let pos = scores.iter().filter(|s| s.1).count();
    let neg = scores.len() - pos;
    if pos == 0 || neg == 0 {
        return None;
    }
    let mut v: Vec<(f64, bool)> = scores.to_vec();
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut rank_sum = 0.0;
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j + 1 < v.len() && v.get(j + 1).map(|x| x.0) == v.get(i).map(|x| x.0) {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            if v.get(k).is_some_and(|x| x.1) {
                rank_sum += avg;
            }
        }
        i = j + 1;
    }
    Some((rank_sum - (pos * (pos + 1)) as f64 / 2.0) / (pos as f64 * neg as f64))
}

// =============================================================================================
// Statistics
// =============================================================================================

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }
}

pub fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
}

/// Quantile with linear interpolation (`q` in 0..=1) of sorted values.
pub fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    let a = sorted.get(lo).copied().unwrap_or(0.0);
    let b = sorted.get(hi).copied().unwrap_or(a);
    a + (b - a) * (pos - lo as f64)
}

pub fn sorted_finite(v: &[f64]) -> Vec<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    s
}

pub fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let pairs: Vec<(f64, f64)> = a.iter().zip(b).filter(|(x, y)| x.is_finite() && y.is_finite()).map(|(x, y)| (*x, *y)).collect();
    let n = pairs.len() as f64;
    if n < 2.0 {
        return f64::NAN;
    }
    let ma = pairs.iter().map(|p| p.0).sum::<f64>() / n;
    let mb = pairs.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut sab, mut saa, mut sbb) = (0.0, 0.0, 0.0);
    for (x, y) in &pairs {
        sab += (x - ma) * (y - mb);
        saa += (x - ma).powi(2);
        sbb += (y - mb).powi(2);
    }
    if saa <= 0.0 || sbb <= 0.0 { f64::NAN } else { sab / (saa * sbb).sqrt() }
}

/// Outlier scores per row: (score, is_outlier).
pub fn outliers(x: &Matrix, method: &str, threshold: f64, seed: u64) -> Vec<(f64, bool)> {
    let n = x.len();
    let d = x.first().map(|r| r.len()).unwrap_or(0);
    match method {
        "iqr" => {
            let bounds: Vec<(f64, f64)> = (0..d)
                .map(|j| {
                    let col: Vec<f64> = x.iter().map(|r| r.get(j).copied().unwrap_or(f64::NAN)).collect();
                    let s = sorted_finite(&col);
                    let q1 = quantile_sorted(&s, 0.25);
                    let q3 = quantile_sorted(&s, 0.75);
                    (q1, q3 - q1)
                })
                .collect();
            x.iter()
                .map(|r| {
                    let mut worst: f64 = 0.0;
                    for (j, v) in r.iter().enumerate() {
                        let (q1, iqr) = bounds.get(j).copied().unwrap_or((0.0, 0.0));
                        if !v.is_finite() || iqr <= 0.0 {
                            continue;
                        }
                        let q3 = q1 + iqr;
                        let dist = if *v < q1 { (q1 - v) / iqr } else if *v > q3 { (v - q3) / iqr } else { 0.0 };
                        worst = worst.max(dist);
                    }
                    (worst, worst > threshold)
                })
                .collect()
        }
        "isolation" => {
            // An isolation forest: random axis-aligned splits; outliers isolate in fewer steps.
            let mut rng = Rng::new(seed);
            let trees = 100;
            let sample = n.min(256);
            let height_limit = (sample.max(2) as f64).log2().ceil() as usize;
            let c = |m: usize| if m <= 1 { 0.0 } else { 2.0 * ((m - 1) as f64).ln() + 0.577_215_664_9 - 2.0 * (m - 1) as f64 / m as f64 };
            let mut paths = vec![0.0; n];
            for _ in 0..trees {
                let mut idx: Vec<usize> = (0..n).collect();
                rng.shuffle(&mut idx);
                idx.truncate(sample);
                // Build the tree as a list of (feature, threshold) splits recursively applied.
                #[derive(Clone)]
                enum INode {
                    Leaf(usize),
                    Split(usize, f64, usize, usize),
                }
                let mut nodes: Vec<INode> = Vec::new();
                fn build(x: &Matrix, idx: Vec<usize>, depth: usize, limit: usize, rng: &mut Rng, nodes: &mut Vec<INode>) -> usize {
                    let me = nodes.len();
                    nodes.push(INode::Leaf(idx.len()));
                    let d = x.first().map(|r| r.len()).unwrap_or(0);
                    if depth >= limit || idx.len() <= 1 || d == 0 {
                        return me;
                    }
                    let f = rng.below(d);
                    let vals: Vec<f64> = idx.iter().filter_map(|i| x.get(*i).and_then(|r| r.get(f)).copied()).filter(|v| v.is_finite()).collect();
                    let lo = vals.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if !(hi > lo) {
                        return me;
                    }
                    let t = lo + rng.f64() * (hi - lo);
                    let (l, r): (Vec<usize>, Vec<usize>) = idx.iter().partition(|i| x.get(**i).and_then(|row| row.get(f)).copied().unwrap_or(0.0) < t);
                    let li = build(x, l, depth + 1, limit, rng, nodes);
                    let ri = build(x, r, depth + 1, limit, rng, nodes);
                    if let Some(node) = nodes.get_mut(me) {
                        *node = INode::Split(f, t, li, ri);
                    }
                    me
                }
                build(x, idx, 0, height_limit, &mut rng, &mut nodes);
                for (i, p) in paths.iter_mut().enumerate() {
                    let row = x.get(i).map(Vec::as_slice).unwrap_or(&[]);
                    let mut node = 0;
                    let mut depth = 0.0;
                    loop {
                        match nodes.get(node) {
                            Some(INode::Split(f, t, l, r)) => {
                                node = if row.get(*f).copied().unwrap_or(0.0) < *t { *l } else { *r };
                                depth += 1.0;
                            }
                            Some(INode::Leaf(size)) => {
                                depth += c(*size);
                                break;
                            }
                            None => break,
                        }
                    }
                    *p += depth;
                }
            }
            let cn = c(sample).max(1e-9);
            paths
                .iter()
                .map(|p| {
                    let s = 2f64.powf(-(p / trees as f64) / cn);
                    (s, s > threshold)
                })
                .collect()
        }
        _ => {
            // z-score: the largest |z| over the features.
            let sc = Scaler::fit(&x.iter().map(|r| r.iter().map(|v| if v.is_finite() { *v } else { 0.0 }).collect()).collect());
            x.iter()
                .map(|r| {
                    let z = r.iter().enumerate().filter(|(_, v)| v.is_finite()).map(|(j, v)| ((v - sc.mean.get(j).copied().unwrap_or(0.0)) / sc.std.get(j).copied().unwrap_or(1.0)).abs()).fold(0.0, f64::max);
                    (z, z > threshold)
                })
                .collect()
        }
    }
}

/// Feature statistics of every column: one row per column.
pub fn feature_stats(t: &TableData) -> TableData {
    let cols = ["Feature", "Type", "Count", "Missing", "Distinct", "Mean", "Std. dev.", "Min", "Q1", "Median", "Q3", "Max", "Mode"];
    let mut rows = Vec::new();
    for (j, c) in t.columns.iter().enumerate() {
        let ty = t.types.get(j).copied().unwrap_or_default();
        let vals: Vec<&Scalar> = t.rows.iter().map(|r| r.get(j).unwrap_or(&Scalar::Null)).collect();
        let missing = vals.iter().filter(|v| v.is_null()).count();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for v in vals.iter().filter(|v| !v.is_null()) {
            *counts.entry(v.text()).or_default() += 1;
        }
        let mode = counts.iter().max_by_key(|(_, n)| **n).map(|(k, _)| k.clone()).unwrap_or_default();
        let nums: Vec<f64> = vals.iter().filter_map(|v| if let Scalar::Num(n) = v { Some(*n) } else { None }).collect();
        let numeric = !nums.is_empty() && (ty.is_numeric() || ty.is_temporal() || ty == ColType::Any);
        let s = sorted_finite(&nums);
        let num = |x: f64| if numeric && x.is_finite() { Scalar::Num(x) } else { Scalar::Null };
        rows.push(vec![
            Scalar::Text(c.clone()),
            Scalar::Text(ty.label().into()),
            Scalar::Num((vals.len() - missing) as f64),
            Scalar::Num(missing as f64),
            Scalar::Num(counts.len() as f64),
            num(mean(&nums)),
            num(std_dev(&nums)),
            num(s.first().copied().unwrap_or(f64::NAN)),
            num(quantile_sorted(&s, 0.25)),
            num(quantile_sorted(&s, 0.5)),
            num(quantile_sorted(&s, 0.75)),
            num(s.last().copied().unwrap_or(f64::NAN)),
            Scalar::Text(mode),
        ]);
    }
    let types = vec![ColType::Text, ColType::Text, ColType::Integer, ColType::Integer, ColType::Integer, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Number, ColType::Text];
    TableData { name: "Feature statistics".into(), columns: cols.iter().map(|s| s.to_string()).collect(), types, rows }
}

/// Equal-width histogram: (bin start, bin end, count).
pub fn histogram(v: &[f64], bins: usize) -> Vec<(f64, f64, usize)> {
    let s = sorted_finite(v);
    let (Some(lo), Some(hi)) = (s.first().copied(), s.last().copied()) else { return vec![] };
    let bins = bins.clamp(1, 200);
    if hi <= lo {
        return vec![(lo, hi, s.len())];
    }
    let w = (hi - lo) / bins as f64;
    let mut counts = vec![0usize; bins];
    for x in &s {
        let b = (((x - lo) / w) as usize).min(bins - 1);
        if let Some(c) = counts.get_mut(b) {
            *c += 1;
        }
    }
    counts.iter().enumerate().map(|(i, c)| (lo + w * i as f64, lo + w * (i + 1) as f64, *c)).collect()
}

// =============================================================================================
// Tests
// =============================================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn blobs() -> Matrix {
        let mut rng = Rng::new(7);
        let mut x = Vec::new();
        for (cx, cy) in [(0.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            for _ in 0..30 {
                x.push(vec![cx + rng.f64() - 0.5, cy + rng.f64() - 0.5]);
            }
        }
        x
    }

    fn table(cols: &[&str], rows: Vec<Vec<Scalar>>) -> TableData {
        let mut t = TableData { name: "t".into(), columns: cols.iter().map(|s| s.to_string()).collect(), types: vec![], rows };
        t.infer_types();
        t
    }

    #[test]
    fn kmeans_finds_blobs_and_auto_k() {
        let x = blobs();
        let r = kmeans(&x, 3, 1, 100);
        // Each blob gets one label.
        for b in 0..3 {
            let l = r.labels[b * 30];
            assert!(r.labels[b * 30..(b + 1) * 30].iter().all(|v| *v == l));
        }
        assert!(silhouette(&x, &r.labels) > 0.8);
        let (_, k, _) = kmeans_auto(&x, 6, 1);
        assert_eq!(k, 3);
        // Deterministic.
        assert_eq!(kmeans(&x, 3, 1, 100), r);
    }

    #[test]
    fn hierarchical_and_dbscan() {
        let x = blobs();
        let l = hierarchical(&x, 3).unwrap();
        assert_eq!(l[0], 0);
        assert!(l[..30].iter().all(|v| *v == l[0]) && l[30..60].iter().all(|v| *v == l[30]) && l[60..].iter().all(|v| *v == l[60]));
        assert_ne!(l[0], l[30]);
        let mut x2 = x.clone();
        x2.push(vec![50.0, 50.0]);
        let d = dbscan(&x2, 1.5, 4);
        assert_eq!(d[90], None);
        assert_eq!(d.iter().flatten().copied().max(), Some(2));
    }

    #[test]
    fn pca_on_correlated_data() {
        let x: Matrix = (0..50).map(|i| vec![i as f64, 2.0 * i as f64 + 0.01 * ((i * 7) % 5) as f64]).collect();
        let p = pca(&x, 2, true);
        assert!(p.explained[0] > 0.99);
        assert!((p.explained.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        let (vals, _) = jacobi_eigen(&vec![vec![2.0, 1.0], vec![1.0, 2.0]]);
        assert!((vals[0] - 3.0).abs() < 1e-9 && (vals[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn linear_regression_recovers_coefficients() {
        let rows: Vec<Vec<Scalar>> = (0..40).map(|i| {
            let a = i as f64;
            let b = ((i * 13) % 7) as f64;
            vec![Scalar::Num(a), Scalar::Num(b), Scalar::Num(3.0 + 2.0 * a - 0.5 * b)]
        }).collect();
        let t = table(&["a", "b", "y"], rows);
        let m = train(&t, &["a".into(), "b".into()], "y", &TrainParams { kind: "linear".into(), ..TrainParams::default() }).unwrap();
        let Learner::Linear { coef, .. } = &m.learner else { panic!() };
        assert!((coef[0] - 3.0).abs() < 1e-6 && (coef[1] - 2.0).abs() < 1e-6 && (coef[2] + 0.5).abs() < 1e-6);
        assert!((m.predict_row(&[10.0, 2.0]).0 - 22.0).abs() < 1e-6);
    }

    fn two_class() -> TableData {
        let mut rng = Rng::new(3);
        let rows = (0..120)
            .map(|i| {
                let c = i % 2;
                let x = if c == 0 { rng.f64() * 4.0 } else { 3.0 + rng.f64() * 4.0 };
                let y = rng.f64();
                vec![Scalar::Num(x), Scalar::Num(y), Scalar::Text(if c == 0 { "no".into() } else { "yes".into() })]
            })
            .collect();
        table(&["x", "noise", "label"], rows)
    }

    #[test]
    fn classifiers_learn_a_simple_boundary() {
        let t = two_class();
        let feats = vec!["x".to_string(), "noise".to_string()];
        for kind in ["logistic", "tree", "forest", "naive_bayes", "knn"] {
            let m = train(&t, &feats, "label", &TrainParams { kind: kind.into(), trees: 20, ..TrainParams::default() }).unwrap();
            assert_eq!(m.classes, vec!["no", "yes"]);
            let x = matrix(&t, &feats).unwrap();
            let pred: Vec<usize> = x.iter().map(|r| m.predict_row(r).0 as usize).collect();
            let actual: Vec<usize> = (0..120).map(|i| i % 2).collect();
            let probs: Vec<Vec<f64>> = x.iter().map(|r| m.predict_row(r).1).collect();
            let cm = class_metrics(&actual, &pred, 2, Some(&probs));
            assert!(cm.accuracy > 0.8, "{kind}: {}", cm.accuracy);
            assert!(cm.auc.unwrap() > 0.85, "{kind}");
            assert_eq!(cm.confusion.iter().flatten().sum::<usize>(), 120);
        }
        // Clear extremes.
        let m = train(&t, &feats, "label", &TrainParams { kind: "tree".into(), ..TrainParams::default() }).unwrap();
        assert_eq!(m.predict_row(&[0.1, 0.5]).0, 0.0);
        assert_eq!(m.predict_row(&[6.9, 0.5]).0, 1.0);
    }

    #[test]
    fn regression_tree_and_metrics() {
        let rows: Vec<Vec<Scalar>> = (0..60).map(|i| vec![Scalar::Num(i as f64), Scalar::Num(if i < 30 { 1.5 } else { 7.25 })]).collect();
        let t = table(&["x", "y"], rows);
        let m = train(&t, &["x".into()], "y", &TrainParams { kind: "tree".into(), task: "regression".into(), ..TrainParams::default() }).unwrap();
        assert!(!m.is_classifier());
        assert!((m.predict_row(&[5.0]).0 - 1.5).abs() < 1e-9);
        assert!((m.predict_row(&[50.0]).0 - 7.25).abs() < 1e-9);
        let r = regression_metrics(&[1.0, 2.0, 3.0], &[1.0, 2.0, 4.0]);
        assert!((r.rmse - (1.0f64 / 3.0).sqrt()).abs() < 1e-12);
        assert!((r.mae - 1.0 / 3.0).abs() < 1e-12);
        assert!((r.r2 - 0.5).abs() < 1e-12);
    }

    #[test]
    fn auc_and_stats() {
        assert_eq!(auc_binary(&[(0.1, false), (0.4, false), (0.35, true), (0.8, true)]), Some(0.75));
        let s = sorted_finite(&[4.0, 1.0, 3.0, 2.0, f64::NAN]);
        assert_eq!(quantile_sorted(&s, 0.5), 2.5);
        assert!((pearson(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]) - 1.0).abs() < 1e-12);
        let h = histogram(&[0.0, 1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(h.iter().map(|b| b.2).collect::<Vec<_>>(), vec![2, 3]);
        let mut x: Matrix = (0..50).map(|i| vec![(i % 10) as f64]).collect();
        x.push(vec![100.0]);
        for (m, th) in [("zscore", 3.0), ("iqr", 1.5), ("isolation", 0.6)] {
            let o = outliers(&x, m, th, 1);
            assert!(o[50].1, "{m}");
            assert!(o[..50].iter().filter(|v| v.1).count() <= 2, "{m}");
        }
    }

    #[test]
    fn invert_matrix() {
        let inv = invert(&vec![vec![4.0, 7.0], vec![2.0, 6.0]]).unwrap();
        assert!((inv[0][0] - 0.6).abs() < 1e-12 && (inv[0][1] + 0.7).abs() < 1e-12);
        assert!(invert(&vec![vec![1.0, 2.0], vec![2.0, 4.0]]).is_none());
    }
}
