// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of the vector quantization parts of `encoder/basisu_enc.h`: `priority_queue`,
//! `tree_vector_quant` (tree structured VQ with PCA splits and k-means refinement),
//! `compute_pca_from_covar` and `generate_hierarchical_codebook_threaded`.
//!
//! Training vectors are `[f32; N]` (`vec<N, float>`); float evaluation order follows the C++
//! code (f32 dot products, f64 accumulators where the C++ uses `double`). The SSE 4.1
//! `update_covar_matrix_16x16` kernel computes the same per element products as the scalar
//! covariance loop, so only the scalar loop is ported.

#![allow(clippy::needless_range_loop)]

use std::cmp::Ordering;
use std::collections::BTreeMap;

/// `vec<N, float>`.
pub type VecNF<const N: usize> = [f32; N];

#[inline]
fn dot<const N: usize>(a: &VecNF<N>, b: &VecNF<N>) -> f32 {
    let mut r = a[0] * b[0];
    for i in 1..N {
        r += a[i] * b[i];
    }
    r
}

#[inline]
fn sub<const N: usize>(a: &VecNF<N>, b: &VecNF<N>) -> VecNF<N> {
    let mut r = [0.0; N];
    for i in 0..N {
        r[i] = a[i] - b[i];
    }
    r
}

#[inline]
fn scale<const N: usize>(a: &VecNF<N>, s: f32) -> VecNF<N> {
    let mut r = [0.0; N];
    for i in 0..N {
        r[i] = a[i] * s;
    }
    r
}

#[inline]
fn add_assign<const N: usize>(a: &mut VecNF<N>, b: &VecNF<N>) {
    for i in 0..N {
        a[i] += b[i];
    }
}

#[inline]
fn scale_assign<const N: usize>(a: &mut VecNF<N>, s: f32) {
    for i in 0..N {
        a[i] *= s;
    }
}

#[inline]
fn squared_distance_d<const N: usize>(a: &VecNF<N>, b: &VecNF<N>) -> f64 {
    let mut d2 = 0.0f64;
    for i in 0..N {
        let d = a[i] as f64 - b[i] as f64;
        d2 += d * d;
    }
    d2
}

/// `vec::operator<` (lexicographic, `-0.0 == 0.0`) as a total order for `std::map` keys.
fn vec_cmp<const N: usize>(a: &VecNF<N>, b: &VecNF<N>) -> Ordering {
    for i in 0..N {
        if a[i] < b[i] {
            return Ordering::Less;
        } else if a[i] != b[i] {
            return Ordering::Greater;
        }
    }
    Ordering::Equal
}

#[derive(Clone, Copy)]
struct VecKey<const N: usize>(VecNF<N>);

impl<const N: usize> PartialEq for VecKey<N> {
    fn eq(&self, o: &Self) -> bool {
        vec_cmp(&self.0, &o.0) == Ordering::Equal
    }
}
impl<const N: usize> Eq for VecKey<N> {}
impl<const N: usize> PartialOrd for VecKey<N> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<const N: usize> Ord for VecKey<N> {
    fn cmp(&self, o: &Self) -> Ordering {
        vec_cmp(&self.0, &o.0)
    }
}

/// `compute_pca_from_covar`: principal axis by 8 power iterations, normalized.
pub fn compute_pca_from_covar<const N: usize>(cmatrix: &[[f32; N]; N]) -> VecNF<N> {
    let mut axis = [0.0f32; N];
    if N == 1 {
        axis[0] = 1.0;
    } else {
        let d = 1.0f32 / (N as i32 - 1).max(1) as f32;
        for i in 0..N {
            // lerp(.75f, 1.25f, i * (1.0f / max(N - 1, 1)))
            axis[i] = 0.75 + (1.25 - 0.75) * (i as f32 * d);
        }
    }
    let mut prev_axis = axis;
    // Power iterations
    for _ in 0..8 {
        let mut trial_axis = [0.0f32; N];
        let mut max_sum = 0.0f64;
        for i in 0..N {
            let mut sum = 0.0f64;
            for j in 0..N {
                sum += (cmatrix[i][j] * axis[j]) as f64;
            }
            trial_axis[i] = sum as f32;
            max_sum = if sum.abs() > max_sum {
                sum.abs()
            } else {
                max_sum
            };
        }
        if max_sum != 0.0 {
            scale_assign(&mut trial_axis, (1.0 / max_sum) as f32);
        }
        let delta_axis = sub(&prev_axis, &trial_axis);
        prev_axis = axis;
        axis = trial_axis;
        if dot(&delta_axis, &delta_axis) < 0.0024 {
            break;
        }
    }
    // normalize_in_place
    let len = dot(&axis, &axis).sqrt();
    if len != 0.0 {
        scale_assign(&mut axis, 1.0 / len);
    }
    axis
}

#[derive(Clone, Copy, Default)]
struct HeapEntry {
    index: u32,
    priority: f32,
}

/// `priority_queue`: max heap on priority (1 based).
#[derive(Default)]
pub struct PriorityQueue {
    heap: Vec<HeapEntry>,
    size: u32,
}

impl PriorityQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.heap.clear();
        self.size = 0;
    }

    pub fn init(&mut self, max_entries: u32, first_index: u32, first_priority: f32) {
        self.heap
            .resize((max_entries + 1).max(2) as usize, HeapEntry::default());
        self.heap[1] = HeapEntry {
            index: first_index,
            priority: first_priority,
        };
        self.size = 1;
    }

    pub fn size(&self) -> u32 {
        self.size
    }

    pub fn get_top_index(&self) -> u32 {
        self.heap.get(1).map_or(0, |e| e.index)
    }

    pub fn get_top_priority(&self) -> f32 {
        self.heap.get(1).map_or(0.0, |e| e.priority)
    }

    pub fn delete_top(&mut self) {
        if self.size == 0 {
            return;
        }
        self.heap[1] = self.heap[self.size as usize];
        self.size -= 1;
        if self.size != 0 {
            self.down_heap(1);
        }
    }

    pub fn add_heap(&mut self, index: u32, priority: f32) {
        self.size += 1;
        let mut k = self.size as usize;
        if self.size as usize >= self.heap.len() {
            self.heap
                .resize(self.size as usize + 1, HeapEntry::default());
        }
        loop {
            let parent_index = k >> 1;
            if parent_index == 0 || self.heap[parent_index].priority > priority {
                break;
            }
            self.heap[k] = self.heap[parent_index];
            k = parent_index;
        }
        self.heap[k] = HeapEntry { index, priority };
    }

    fn down_heap(&mut self, mut heap_index: usize) {
        let orig = self.heap[heap_index];
        let size = self.size as usize;
        loop {
            let mut child_index = heap_index << 1;
            if child_index > size {
                break;
            }
            if child_index < size
                && self.heap[child_index].priority < self.heap[child_index + 1].priority
            {
                child_index += 1;
            }
            if orig.priority > self.heap[child_index].priority {
                break;
            }
            self.heap[heap_index] = self.heap[child_index];
            heap_index = child_index;
        }
        self.heap[heap_index] = orig;
    }
}

#[derive(Clone)]
struct TsvqNode<const N: usize> {
    var: f32,
    weight: u64,
    origin: VecNF<N>,
    left_index: i32,
    right_index: i32,
    training_vecs: Vec<u32>,
    codebook_index: i32,
}

impl<const N: usize> Default for TsvqNode<N> {
    fn default() -> Self {
        TsvqNode {
            var: 0.0,
            weight: 0,
            origin: [0.0; N],
            left_index: -1,
            right_index: -1,
            training_vecs: Vec::new(),
            codebook_index: -1,
        }
    }
}

impl<const N: usize> TsvqNode<N> {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.left_index < 0
    }
}

/// Result of `refine_split` for one child.
struct Child<const N: usize> {
    org: VecNF<N>,
    weight: u64,
    var: f32,
    children: Vec<u32>,
}

/// `tree_vector_quant<vec<N, float>>`: tree structured vector quantization (TSVQ).
#[derive(Clone, Default)]
pub struct TreeVectorQuant<const N: usize> {
    nodes: Vec<TsvqNode<N>>,
    training_vecs: Vec<(VecNF<N>, u64)>,
    next_codebook_index: u32,
}

impl<const N: usize> TreeVectorQuant<N> {
    pub fn new() -> Self {
        TreeVectorQuant {
            nodes: Vec::new(),
            training_vecs: Vec::new(),
            next_codebook_index: 0,
        }
    }

    pub fn clear(&mut self) {
        self.training_vecs = Vec::new();
        self.nodes = Vec::new();
        self.next_codebook_index = 0;
    }

    pub fn add_training_vec(&mut self, v: &VecNF<N>, weight: u64) {
        self.training_vecs.push((*v, weight));
    }

    pub fn get_total_training_vecs(&self) -> usize {
        self.training_vecs.len()
    }

    pub fn get_training_vecs(&self) -> &[(VecNF<N>, u64)] {
        &self.training_vecs
    }

    pub fn get_training_vecs_mut(&mut self) -> &mut Vec<(VecNF<N>, u64)> {
        &mut self.training_vecs
    }

    /// `retrieve(vector<vector<uint32_t>>&)`: appends the training vector indices of each leaf.
    pub fn retrieve(&self, codebook: &mut Vec<Vec<u32>>) {
        for n in &self.nodes {
            if n.is_leaf() {
                codebook.push(n.training_vecs.clone());
            }
        }
    }

    /// `retrieve(vector<TrainingVectorType>&)`: appends the origin of each leaf.
    pub fn retrieve_origins(&self, codebook: &mut Vec<VecNF<N>>) {
        for n in &self.nodes {
            if n.is_leaf() {
                codebook.push(n.origin);
            }
        }
    }

    /// `retrieve(max_clusters, vector<uint_vec>&)`: the clusters of the tree cut at
    /// `max_clusters` (depth first, left to right).
    pub fn retrieve_max(&self, max_clusters: u32, codebook: &mut Vec<Vec<u32>>) {
        codebook.clear();
        if self.nodes.is_empty() {
            return;
        }
        let mut node_stack: Vec<u32> = Vec::with_capacity(512);
        let mut node_index = 0usize;
        loop {
            let cur = &self.nodes[node_index];
            if cur.is_leaf() || 2 + cur.codebook_index > max_clusters as i32 {
                codebook.push(cur.training_vecs.clone());
                match node_stack.pop() {
                    Some(n) => node_index = n as usize,
                    None => break,
                }
                continue;
            }
            node_stack.push(cur.right_index as u32);
            node_index = cur.left_index as usize;
        }
    }

    pub fn generate(&mut self, max_size: u32) -> bool {
        if self.training_vecs.is_empty() {
            return false;
        }
        self.next_codebook_index = 0;
        self.nodes = Vec::with_capacity(max_size as usize * 2 + 1);
        let root = self.prepare_root();
        let root_var = root.var;
        self.nodes.push(root);

        let mut var_heap = PriorityQueue::new();
        var_heap.init(max_size, 0, root_var);

        let mut total_leaf_nodes = 1u32;
        while var_heap.size() != 0 && total_leaf_nodes < max_size {
            let node_index = var_heap.get_top_index();
            var_heap.delete_top();
            if self.nodes[node_index as usize].training_vecs.len() > 1
                && self.split_node(node_index as usize, &mut var_heap)
            {
                // This removes one leaf node (making an internal node) and replaces it with
                // two new leaves, so +1 total.
                total_leaf_nodes += 1;
            }
        }
        true
    }

    fn prepare_root(&self) -> TsvqNode<N> {
        let mut ttsum = 0.0f64;
        let mut root = TsvqNode::<N> {
            training_vecs: Vec::with_capacity(self.training_vecs.len()),
            ..Default::default()
        };
        for (i, (v, weight)) in self.training_vecs.iter().enumerate() {
            root.training_vecs.push(i as u32);
            add_assign(&mut root.origin, &scale(v, *weight as f32));
            root.weight += *weight;
            ttsum += (dot(v, v) * *weight as f32) as f64;
        }
        root.var = (ttsum - (dot(&root.origin, &root.origin) / root.weight as f32) as f64) as f32;
        scale_assign(&mut root.origin, 1.0 / root.weight as f32);
        root
    }

    fn split_node(&mut self, node_index: usize, var_heap: &mut PriorityQueue) -> bool {
        // Compute initial left/right child origins
        let Some((l_org, r_org)) = self.prep_split(&self.nodes[node_index]) else {
            return false;
        };
        // Use k-means iterations to refine these children vectors
        let Some((l, r)) = self.refine_split(&self.nodes[node_index], l_org, r_org) else {
            return false;
        };

        // Create children
        let l_child_index = self.nodes.len();
        let r_child_index = l_child_index + 1;
        {
            let n = &mut self.nodes[node_index];
            n.left_index = l_child_index as i32;
            n.right_index = r_child_index as i32;
            n.codebook_index = self.next_codebook_index as i32;
        }
        self.next_codebook_index += 1;

        let mut kids = [l, r].map(|c| TsvqNode::<N> {
            var: c.var,
            weight: c.weight,
            origin: c.org,
            training_vecs: c.children,
            ..Default::default()
        });
        for k in kids.iter_mut() {
            if k.var <= 0.0 && k.training_vecs.len() > 1 {
                let v = self.training_vecs[k.training_vecs[0] as usize].0;
                for &t in &k.training_vecs[1..] {
                    if v != self.training_vecs[t as usize].0 {
                        k.var = 1e-4;
                        break;
                    }
                }
            }
        }
        let [l, r] = kids;
        let (lv, ln) = (l.var, l.training_vecs.len());
        let (rv, rn) = (r.var, r.training_vecs.len());
        self.nodes.push(l);
        self.nodes.push(r);
        if lv > 0.0 && ln > 1 {
            var_heap.add_heap(l_child_index as u32, lv);
        }
        if rv > 0.0 && rn > 1 {
            var_heap.add_heap(r_child_index as u32, rv);
        }
        true
    }

    fn compute_split_axis(&self, node: &TsvqNode<N>) -> VecNF<N> {
        let mut cmatrix = [[0.0f32; N]; N];
        // Compute covariance matrix from weighted input vectors
        for &t in &node.training_vecs {
            let (tv, tw) = &self.training_vecs[t as usize];
            let v = sub(tv, &node.origin);
            let w = scale(&v, *tw as f32);
            for x in 0..N {
                for y in x..N {
                    cmatrix[x][y] += v[x] * w[y];
                }
            }
        }
        let renorm_scale = 1.0f32 / node.weight as f32;
        for x in 0..N {
            for y in x..N {
                cmatrix[x][y] *= renorm_scale;
            }
        }
        // Diagonal flip
        for x in 0..N.saturating_sub(1) {
            for y in x + 1..N {
                cmatrix[y][x] = cmatrix[x][y];
            }
        }
        compute_pca_from_covar(&cmatrix)
    }

    fn prep_split(&self, node: &TsvqNode<N>) -> Option<(VecNF<N>, VecNF<N>)> {
        let tv = &self.training_vecs;
        if node.training_vecs.len() == 2 {
            return Some((
                tv[node.training_vecs[0] as usize].0,
                tv[node.training_vecs[1] as usize].0,
            ));
        }
        let axis = self.compute_split_axis(node);
        let (mut l_child, mut r_child) = ([0.0f32; N], [0.0f32; N]);
        let (mut l_weight, mut r_weight) = (0.0f64, 0.0f64);

        // Compute initial left/right children
        for &i in &node.training_vecs {
            let (v, w) = &tv[i as usize];
            let weight = *w as f32;
            let t = dot(&sub(v, &node.origin), &axis) as f64;
            if t >= 0.0 {
                add_assign(&mut r_child, &scale(v, weight));
                r_weight += weight as f64;
            } else {
                add_assign(&mut l_child, &scale(v, weight));
                l_weight += weight as f64;
            }
        }
        if l_weight > 0.0 && r_weight > 0.0 {
            return Some((
                scale(&l_child, (1.0 / l_weight) as f32),
                scale(&r_child, (1.0 / r_weight) as f32),
            ));
        }

        let mut l = [1e20f32; N];
        let mut h = [-1e20f32; N];
        for &i in &node.training_vecs {
            let v = &tv[i as usize].0;
            for c in 0..N {
                l[c] = if l[c] < v[c] { l[c] } else { v[c] };
                h[c] = if h[c] > v[c] { h[c] } else { v[c] };
            }
        }
        let r = sub(&h, &l);
        let mut largest_axis_v = 0.0f32;
        let mut largest_axis_index = -1i32;
        for i in 0..N {
            if r[i] > largest_axis_v {
                largest_axis_v = r[i];
                largest_axis_index = i as i32;
            }
        }
        if largest_axis_index < 0 {
            return None;
        }
        // C++ sorts the vectors along the largest axis here but then splits by the unsorted
        // order (the sorted indices are unused); kept as is.
        l_child = [0.0; N];
        l_weight = 0.0;
        r_child = [0.0; N];
        r_weight = 0.0;
        let half_index = node.training_vecs.len() / 2;
        for (i, &ti) in node.training_vecs.iter().enumerate() {
            let (v, w) = &tv[ti as usize];
            let weight = *w as f32;
            if i < half_index {
                add_assign(&mut l_child, &scale(v, weight));
                l_weight += weight as f64;
            } else {
                add_assign(&mut r_child, &scale(v, weight));
                r_weight += weight as f64;
            }
        }
        if l_weight > 0.0 && r_weight > 0.0 {
            Some((
                scale(&l_child, (1.0 / l_weight) as f32),
                scale(&r_child, (1.0 / r_weight) as f32),
            ))
        } else {
            Some((l, h))
        }
    }

    fn refine_split(
        &self,
        node: &TsvqNode<N>,
        mut l_child: VecNF<N>,
        mut r_child: VecNF<N>,
    ) -> Option<(Child<N>, Child<N>)> {
        let tv = &self.training_vecs;
        let n = node.training_vecs.len();
        let mut l_children: Vec<u32> = Vec::with_capacity(n);
        let mut r_children: Vec<u32> = Vec::with_capacity(n);
        let (mut l_weight, mut r_weight) = (0u64, 0u64);
        let (mut l_var, mut r_var) = (0.0f32, 0.0f32);
        let mut prev_total_variance = 1e10f32;

        // Refine left/right children locations using k-means iterations
        const MAX_ITERS: u32 = 6;
        for _ in 0..MAX_ITERS {
            l_children.clear();
            r_children.clear();
            let (mut new_l_child, mut new_r_child) = ([0.0f32; N], [0.0f32; N]);
            let (mut l_ttsum, mut r_ttsum) = (0.0f64, 0.0f64);
            l_weight = 0;
            r_weight = 0;

            for &ti in &node.training_vecs {
                let (v, weight) = &tv[ti as usize];
                let left_dist2 = squared_distance_d(&l_child, v);
                let right_dist2 = squared_distance_d(&r_child, v);
                if left_dist2 >= right_dist2 {
                    add_assign(&mut new_r_child, &scale(v, *weight as f32));
                    r_weight += *weight;
                    r_ttsum += (*weight as f32 * dot(v, v)) as f64;
                    r_children.push(ti);
                } else {
                    add_assign(&mut new_l_child, &scale(v, *weight as f32));
                    l_weight += *weight;
                    l_ttsum += (*weight as f32 * dot(v, v)) as f64;
                    l_children.push(ti);
                }
            }

            // Node is unsplittable using the above algorithm - try something else to split it up.
            if l_weight == 0 || r_weight == 0 {
                l_children.clear();
                new_l_child = [0.0; N];
                l_ttsum = 0.0;
                l_weight = 0;
                r_children.clear();
                new_r_child = [0.0; N];
                r_ttsum = 0.0;
                r_weight = 0;

                let mut first_vec = [0.0f32; N];
                for (i, &ti) in node.training_vecs.iter().enumerate() {
                    let (v, weight) = &tv[ti as usize];
                    if i == 0 || *v == first_vec {
                        first_vec = *v;
                        add_assign(&mut new_r_child, &scale(v, *weight as f32));
                        r_weight += *weight;
                        r_ttsum += (*weight as f32 * dot(v, v)) as f64;
                        r_children.push(ti);
                    } else {
                        add_assign(&mut new_l_child, &scale(v, *weight as f32));
                        l_weight += *weight;
                        l_ttsum += (*weight as f32 * dot(v, v)) as f64;
                        l_children.push(ti);
                    }
                }
                if l_weight == 0 || r_weight == 0 {
                    return None;
                }
            }

            l_var = (l_ttsum - (dot(&new_l_child, &new_l_child) / l_weight as f32) as f64) as f32;
            r_var = (r_ttsum - (dot(&new_r_child, &new_r_child) / r_weight as f32) as f64) as f32;

            scale_assign(&mut new_l_child, 1.0 / l_weight as f32);
            scale_assign(&mut new_r_child, 1.0 / r_weight as f32);
            l_child = new_l_child;
            r_child = new_r_child;

            let total_var = l_var + r_var;
            const GIVEUP_VARIANCE: f32 = 0.00001;
            if total_var < GIVEUP_VARIANCE {
                break;
            }
            // Check to see if the variance has settled
            const VARIANCE_DELTA_THRESH: f32 = 0.00125;
            if (prev_total_variance - total_var) / total_var < VARIANCE_DELTA_THRESH {
                break;
            }
            prev_total_variance = total_var;
        }

        Some((
            Child {
                org: l_child,
                weight: l_weight,
                var: l_var,
                children: l_children,
            },
            Child {
                org: r_child,
                weight: r_weight,
                var: r_var,
                children: r_children,
            },
        ))
    }
}

const MAX_THREADS: u32 = 16;

/// Clusters and parent clusters of one clusterizer job.
type JobClusters = (Vec<Vec<u32>>, Vec<Vec<u32>>);

/// `generate_hierarchical_codebook_threaded_internal`. The C++ job pool runs one independent
/// quantizer per initial cluster; here they run on scoped threads (same result).
fn generate_hierarchical_codebook_threaded_internal<const N: usize>(
    q: &mut TreeVectorQuant<N>,
    max_codebook_size: u32,
    max_parent_codebook_size: u32,
    codebook: &mut Vec<Vec<u32>>,
    parent_codebook: &mut Vec<Vec<u32>>,
    mut max_threads: u32,
    limit_clusterizers: bool,
) -> bool {
    codebook.clear();
    parent_codebook.clear();

    if max_threads <= 1 || q.get_training_vecs().len() < 256 || max_codebook_size < max_threads * 16
    {
        if !q.generate(max_codebook_size) {
            return false;
        }
        q.retrieve(codebook);
        if max_parent_codebook_size != 0 {
            q.retrieve_max(max_parent_codebook_size, parent_codebook);
        }
        return true;
    }

    max_threads = max_threads.min(MAX_THREADS);
    if !q.generate(max_threads) {
        return false;
    }
    let mut initial_codebook: Vec<Vec<u32>> = Vec::new();
    q.retrieve(&mut initial_codebook);
    if (initial_codebook.len() as u32) < max_threads {
        *codebook = initial_codebook;
        if max_parent_codebook_size != 0 {
            q.retrieve_max(max_parent_codebook_size, parent_codebook);
        }
        return true;
    }

    let q: &TreeVectorQuant<N> = q;
    let job = |cluster_indices: &Vec<u32>| -> Option<JobClusters> {
        let mut lq = TreeVectorQuant::<N>::new();
        for &g in cluster_indices {
            let (v, w) = &q.get_training_vecs()[g as usize];
            lq.add_training_vec(v, *w);
        }
        let max_clusters = if limit_clusterizers {
            max_codebook_size.div_ceil(max_threads)
        } else {
            lq.get_total_training_vecs() as u32
        };
        if !lq.generate(max_clusters) {
            return None;
        }
        let mut local_clusters = Vec::new();
        lq.retrieve(&mut local_clusters);
        for c in local_clusters.iter_mut() {
            for j in c.iter_mut() {
                *j = cluster_indices[*j as usize];
            }
        }
        let mut local_parent_clusters = Vec::new();
        if max_parent_codebook_size != 0 {
            lq.retrieve_max(
                max_parent_codebook_size.div_ceil(max_threads),
                &mut local_parent_clusters,
            );
            for c in local_parent_clusters.iter_mut() {
                for j in c.iter_mut() {
                    *j = cluster_indices[*j as usize];
                }
            }
        }
        Some((local_clusters, local_parent_clusters))
    };

    let results: Vec<Option<JobClusters>> = std::thread::scope(|s| {
        let handles: Vec<_> = initial_codebook[..max_threads as usize]
            .iter()
            .map(|c| s.spawn(move || job(c)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect()
    });

    let mut all = Vec::with_capacity(results.len());
    for r in results {
        match r {
            Some(r) => all.push(r),
            None => return false,
        }
    }
    for (lc, lp) in all {
        codebook.extend(lc);
        parent_codebook.extend(lp);
    }
    true
}

/// `generate_hierarchical_codebook_threaded`: clusters `q`'s training vectors into at most
/// `max_codebook_size` clusters (and `max_parent_codebook_size` parent clusters when non
/// zero), after merging identical vectors (ordered like the C++ `std::map`). With
/// `even_odd_input_pairs_equal`, vectors `2i` and `2i + 1` are known equal and grouped together.
/// Clusters are returned as lists of training vector indices.
pub fn generate_hierarchical_codebook_threaded<const N: usize>(
    q: &mut TreeVectorQuant<N>,
    max_codebook_size: u32,
    max_parent_codebook_size: u32,
    codebook: &mut Vec<Vec<u32>>,
    parent_codebook: &mut Vec<Vec<u32>>,
    max_threads: u32,
    even_odd_input_pairs_equal: bool,
) -> bool {
    // (total weight, training vector indices) per unique vector
    let mut unique_vecs: BTreeMap<VecKey<N>, (u64, Vec<u32>)> = BTreeMap::new();
    let tv = q.get_training_vecs();
    if even_odd_input_pairs_equal {
        let mut i = 0;
        while i + 1 < tv.len() {
            let w = tv[i].1 + tv[i + 1].1;
            let e = unique_vecs
                .entry(VecKey(tv[i].0))
                .or_insert_with(|| (0, Vec::new()));
            e.0 += w;
            e.1.push(i as u32);
            e.1.push(i as u32 + 1);
            i += 2;
        }
    } else {
        for (i, (v, w)) in tv.iter().enumerate() {
            let e = unique_vecs
                .entry(VecKey(*v))
                .or_insert_with(|| (0, Vec::new()));
            e.0 += *w;
            e.1.push(i as u32);
        }
    }

    let mut group_quant = TreeVectorQuant::<N>::new();
    let mut groups: Vec<Vec<u32>> = Vec::with_capacity(unique_vecs.len());
    for (k, (w, idx)) in unique_vecs {
        group_quant.add_training_vec(&k.0, w);
        groups.push(idx);
    }

    let limit_clusterizers = groups.len() > max_codebook_size as usize;

    let mut group_codebook = Vec::new();
    let mut group_parent_codebook = Vec::new();
    let threads = if groups.len() < 65536 * 4 {
        1
    } else {
        max_threads
    };
    if !generate_hierarchical_codebook_threaded_internal(
        &mut group_quant,
        max_codebook_size,
        max_parent_codebook_size,
        &mut group_codebook,
        &mut group_parent_codebook,
        threads,
        limit_clusterizers,
    ) {
        return false;
    }

    let expand = |src: &Vec<Vec<u32>>| -> Vec<Vec<u32>> {
        src.iter()
            .map(|c| {
                let mut out = Vec::new();
                for &g in c {
                    out.extend_from_slice(&groups[g as usize]);
                }
                out
            })
            .collect()
    };
    *codebook = expand(&group_codebook);
    *parent_codebook = expand(&group_parent_codebook);
    true
}
