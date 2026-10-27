// SPDX-FileCopyrightText: 2019-2026 Binomial LLC
// SPDX-License-Identifier: Apache-2.0
// Translated to Rust from Basis Universal and modified; see NOTICE.md.
//! Port of basisu `encoder/basisu_resampler.cpp` (separable 2D resampler).
//!
//! Contributor lists are reference counted so they can be shared between
//! resamplers like `Pclist_x`/`Pclist_y` in the original. Buffers that the
//! original allocates lazily stay lazy; allocation failure maps to
//! `STATUS_OUT_OF_MEMORY` instead of aborting.

use std::rc::Rc;

use super::resample_filters::{G_RESAMPLE_FILTERS, ResampleFilterFunc};

pub type ResampleReal = f32;
pub type Sample = ResampleReal;

pub const BASISU_RESAMPLER_DEFAULT_FILTER: &str = "lanczos4";
pub const BASISU_RESAMPLER_MAX_DIMENSION: i32 = 16384;
const MAX_SCAN_BUF_SIZE: usize = BASISU_RESAMPLER_MAX_DIMENSION as usize;

/// Port of `Resampler::Contrib`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Contrib {
    pub weight: ResampleReal,
    pub pixel: u16,
}

/// Port of `Resampler::Contrib_List`; `p` holds exactly `n` contributors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ContribList {
    pub n: u16,
    pub p: Vec<Contrib>,
}

/// One contributor list per destination sample (the C array of `Contrib_List`).
pub type ContribLists = Rc<Vec<ContribList>>;

/// Port of `Resampler::Boundary_Op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum BoundaryOp {
    Wrap = 0,
    Reflect = 1,
    Clamp = 2,
}

/// Port of `Resampler::Status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Okay = 0,
    OutOfMemory = 1,
    BadFilterName = 2,
    ScanBufferFull = 3,
}

fn posmod(x: i32, y: i32) -> i32 {
    if y == 0 {
        return 0;
    }
    if x >= 0 {
        return if x < y { x } else { x.wrapping_rem(y) };
    }
    let m = x.wrapping_neg().wrapping_rem(y);
    if m != 0 { y.wrapping_sub(m) } else { m }
}

fn try_vec<T: Clone>(n: usize, v: T) -> Option<Vec<T>> {
    let mut out = Vec::new();
    out.try_reserve_exact(n).ok()?;
    out.resize(n, v);
    Some(out)
}

struct ScanBuf {
    scan_buf_y: Vec<i32>,
    scan_buf_l: Vec<Option<Vec<Sample>>>,
}

/// Port of `basisu::Resampler`.
pub struct Resampler {
    intermediate_x: i32,
    resample_src_x: i32,
    resample_src_y: i32,
    resample_dst_x: i32,
    resample_dst_y: i32,
    boundary_op: BoundaryOp,
    dst_buf: Vec<Sample>,
    tmp_buf: Vec<Sample>,
    clist_x: Option<ContribLists>,
    clist_y: Option<ContribLists>,
    clist_x_forced: bool,
    clist_y_forced: bool,
    delay_x_resample: bool,
    src_y_count: Vec<i32>,
    src_y_flag: Vec<u8>,
    scan_buf: Option<ScanBuf>,
    cur_src_y: i32,
    cur_dst_y: i32,
    status: Status,
    lo: ResampleReal,
    hi: ResampleReal,
}

impl Resampler {
    /// `src_x`/`src_y` - input dimensions, `dst_x`/`dst_y` - output dimensions.
    /// Output samples are clamped to `[sample_low, sample_high]` unless
    /// `sample_low >= sample_high`. `pclist_x`/`pclist_y` optionally share the
    /// contributor lists of another resampler. `None` filter selects the default.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        src_x: i32,
        src_y: i32,
        dst_x: i32,
        dst_y: i32,
        boundary_op: BoundaryOp,
        sample_low: ResampleReal,
        sample_high: ResampleReal,
        filter_name: Option<&str>,
        pclist_x: Option<ContribLists>,
        pclist_y: Option<ContribLists>,
        filter_x_scale: ResampleReal,
        filter_y_scale: ResampleReal,
        src_x_ofs: ResampleReal,
        src_y_ofs: ResampleReal,
    ) -> Resampler {
        let mut r = Resampler {
            intermediate_x: 0,
            resample_src_x: src_x,
            resample_src_y: src_y,
            resample_dst_x: dst_x,
            resample_dst_y: dst_y,
            boundary_op,
            dst_buf: Vec::new(),
            tmp_buf: Vec::new(),
            clist_x: None,
            clist_y: None,
            clist_x_forced: false,
            clist_y_forced: false,
            delay_x_resample: false,
            src_y_count: Vec::new(),
            src_y_flag: Vec::new(),
            scan_buf: None,
            cur_src_y: 0,
            cur_dst_y: 0,
            status: Status::Okay,
            lo: sample_low,
            hi: sample_high,
        };
        r.init(
            pclist_x,
            pclist_y,
            filter_name,
            filter_x_scale,
            filter_y_scale,
            src_x_ofs,
            src_y_ofs,
        );
        r
    }

    #[allow(clippy::too_many_arguments)]
    fn init(
        &mut self,
        pclist_x: Option<ContribLists>,
        pclist_y: Option<ContribLists>,
        filter_name: Option<&str>,
        filter_x_scale: ResampleReal,
        filter_y_scale: ResampleReal,
        src_x_ofs: ResampleReal,
        src_y_ofs: ResampleReal,
    ) {
        let Some(dst_buf) = try_vec(self.resample_dst_x.max(0) as usize, 0.0) else {
            self.status = Status::OutOfMemory;
            return;
        };
        self.dst_buf = dst_buf;

        // Find the specified filter.
        let name = filter_name.unwrap_or(BASISU_RESAMPLER_DEFAULT_FILTER);
        let Some(filter) = G_RESAMPLE_FILTERS.iter().find(|f| f.name == name) else {
            self.status = Status::BadFilterName;
            return;
        };
        let func = filter.func;
        let support = filter.support;

        // Create contributor lists, unless the user supplied custom lists.
        match pclist_x {
            None => {
                self.clist_x = Self::make_clist(
                    self.resample_src_x,
                    self.resample_dst_x,
                    self.boundary_op,
                    func,
                    support,
                    filter_x_scale,
                    src_x_ofs,
                )
                .map(Rc::new);
                if self.clist_x.is_none() {
                    self.status = Status::OutOfMemory;
                    return;
                }
            }
            Some(l) => {
                self.clist_x = Some(l);
                self.clist_x_forced = true;
            }
        }
        match pclist_y {
            None => {
                self.clist_y = Self::make_clist(
                    self.resample_src_y,
                    self.resample_dst_y,
                    self.boundary_op,
                    func,
                    support,
                    filter_y_scale,
                    src_y_ofs,
                )
                .map(Rc::new);
                if self.clist_y.is_none() {
                    self.status = Status::OutOfMemory;
                    return;
                }
            }
            Some(l) => {
                self.clist_y = Some(l);
                self.clist_y_forced = true;
            }
        }

        let src_y = self.resample_src_y.max(0) as usize;
        let (Some(count), Some(flag)) = (try_vec(src_y, 0i32), try_vec(src_y, 0u8)) else {
            self.status = Status::OutOfMemory;
            return;
        };
        self.src_y_count = count;
        self.src_y_flag = flag;

        // Count how many times each source line contributes to a destination line.
        self.count_src_y();

        self.scan_buf = Some(ScanBuf {
            scan_buf_y: vec![-1; MAX_SCAN_BUF_SIZE],
            scan_buf_l: vec![None; MAX_SCAN_BUF_SIZE],
        });

        self.cur_src_y = 0;
        self.cur_dst_y = 0;
        {
            // Determine which axis to resample first by comparing the number of multiplies
            // required for each possibility.
            let x_ops = count_ops(self.clist_x.as_deref(), self.resample_dst_x);
            let y_ops = count_ops(self.clist_y.as_deref(), self.resample_dst_y);

            // Hack 10/2000: Weight Y axis ops a little more than X axis ops.
            let xy_ops = x_ops
                .wrapping_mul(self.resample_src_y)
                .wrapping_add(4i32.wrapping_mul(y_ops).wrapping_mul(self.resample_dst_x) / 3);
            let yx_ops = (4i32.wrapping_mul(y_ops).wrapping_mul(self.resample_src_x) / 3)
                .wrapping_add(x_ops.wrapping_mul(self.resample_dst_y));

            // In case of a tie, choose the order which buffers the least amount of data.
            if xy_ops > yx_ops || (xy_ops == yx_ops && self.resample_src_x < self.resample_dst_x) {
                self.delay_x_resample = true;
                self.intermediate_x = self.resample_src_x;
            } else {
                self.delay_x_resample = false;
                self.intermediate_x = self.resample_dst_x;
            }
        }

        if self.delay_x_resample {
            let Some(tmp) = try_vec(self.intermediate_x.max(0) as usize, 0.0) else {
                self.status = Status::OutOfMemory;
                return;
            };
            self.tmp_buf = tmp;
        }
    }

    fn count_src_y(&mut self) {
        let Some(cy) = self.clist_y.as_deref() else {
            return;
        };
        for l in cy.iter().take(self.resample_dst_y.max(0) as usize) {
            for c in l.p.iter().take(l.n as usize) {
                if let Some(v) = self.src_y_count.get_mut(c.pixel as usize) {
                    *v = v.wrapping_add(1);
                }
            }
        }
    }

    /// Ensure that the contributing source sample is within bounds. If not, reflect, clamp,
    /// or wrap.
    fn reflect(j: i32, src_x: i32, boundary_op: BoundaryOp) -> i32 {
        if j < 0 {
            match boundary_op {
                BoundaryOp::Reflect => {
                    let mut n = j.wrapping_neg();
                    if n >= src_x {
                        n = src_x - 1;
                    }
                    n
                }
                BoundaryOp::Wrap => posmod(j, src_x),
                BoundaryOp::Clamp => 0,
            }
        } else if j >= src_x {
            match boundary_op {
                BoundaryOp::Reflect => {
                    let n = src_x.wrapping_sub(j).wrapping_add(src_x - 1);
                    n.max(0)
                }
                BoundaryOp::Wrap => posmod(j, src_x),
                BoundaryOp::Clamp => src_x - 1,
            }
        } else {
            j
        }
    }

    /// Generates, for all destination samples, the list of all source samples with
    /// non-zero weighted contributions. `None` on failure (the C `NULL`).
    pub fn make_clist(
        src_x: i32,
        dst_x: i32,
        boundary_op: BoundaryOp,
        filter: ResampleFilterFunc,
        filter_support: ResampleReal,
        filter_scale: ResampleReal,
        src_ofs: ResampleReal,
    ) -> Option<Vec<ContribList>> {
        if dst_x < 0 {
            return None;
        }
        let dst = dst_x as usize;
        let mut bounds: Vec<(ResampleReal, i32, i32)> = try_vec(dst, (0.0, 0, 0))?;

        let oo_filter_scale: ResampleReal = 1.0 / filter_scale;
        const NUDGE: ResampleReal = 0.5;
        let xscale = dst_x as ResampleReal / src_x as ResampleReal;
        let downsample = xscale < 1.0;

        // stretched half width of filter when downsampling
        let half_width = if downsample {
            (filter_support / xscale) * filter_scale
        } else {
            filter_support * filter_scale
        };

        // Find the range of source sample(s) that will contribute to each destination sample.
        let mut n: i32 = 0;
        for (i, b) in bounds.iter_mut().enumerate() {
            // Convert from discrete to continuous coordinates, scale, then convert back.
            let mut center = (i as ResampleReal + NUDGE) / xscale;
            center -= NUDGE;
            center += src_ofs;
            let left = (center - half_width).floor() as i32;
            let right = (center + half_width).ceil() as i32;
            *b = (center, left, right);
            n = n.wrapping_add(right.wrapping_sub(left).wrapping_add(1));
        }

        // Allocate memory for contributors.
        if n <= 0 {
            return None;
        }
        let mut pool_left = n as usize;
        let mut out: Vec<ContribList> = Vec::new();
        out.try_reserve_exact(dst).ok()?;

        for &(center, left, right) in &bounds {
            let mut max_k: i32 = -1;
            let mut max_w: ResampleReal = -1e+20;

            let span = right.wrapping_sub(left).wrapping_add(1) as usize;
            pool_left = pool_left.checked_sub(span)?;

            let weight_at = |j: i32| -> ResampleReal {
                if downsample {
                    filter((center - j as ResampleReal) * xscale * oo_filter_scale)
                } else {
                    filter((center - j as ResampleReal) * oo_filter_scale)
                }
            };

            let mut total_weight: ResampleReal = 0.0;
            for j in left..=right {
                total_weight += weight_at(j);
            }
            let norm: ResampleReal = 1.0 / total_weight;

            total_weight = 0.0;
            let mut list = ContribList::default();
            for j in left..=right {
                let weight = weight_at(j) * norm;
                if weight == 0.0 {
                    continue;
                }
                let n = Self::reflect(j, src_x, boundary_op);

                // Increment the number of source samples which contribute to the current
                // destination sample.
                let k = list.n as i32;
                list.n = list.n.wrapping_add(1);
                list.p.push(Contrib {
                    weight,
                    pixel: n as u16,
                });

                total_weight += weight;
                if weight > max_w {
                    max_w = weight;
                    max_k = k;
                }
            }

            if max_k == -1 || list.n == 0 {
                return None;
            }
            if total_weight != 1.0
                && let Some(c) = list.p.get_mut(max_k as usize)
            {
                c.weight += 1.0 - total_weight;
            }
            out.push(list);
        }
        Some(out)
    }

    fn resample_x(&self, dst: &mut [Sample], src: &[Sample]) {
        let Some(cx) = self.clist_x.as_deref() else {
            return;
        };
        for (d, l) in dst
            .iter_mut()
            .zip(cx.iter())
            .take(self.resample_dst_x.max(0) as usize)
        {
            let mut total: Sample = 0.0;
            for c in l.p.iter().take(l.n as usize) {
                total += src.get(c.pixel as usize).copied().unwrap_or(0.0) * c.weight;
            }
            *d = total;
        }
    }

    fn scale_y_mov(tmp: &mut [Sample], src: &[Sample], weight: ResampleReal, dst_x: i32) {
        // Not += because temp buf wasn't cleared.
        for (t, s) in tmp.iter_mut().zip(src).take(dst_x.max(0) as usize) {
            *t = *s * weight;
        }
    }

    fn scale_y_add(tmp: &mut [Sample], src: &[Sample], weight: ResampleReal, dst_x: i32) {
        for (t, s) in tmp.iter_mut().zip(src).take(dst_x.max(0) as usize) {
            *t += *s * weight;
        }
    }

    fn clamp_sample(&self, mut f: ResampleReal) -> ResampleReal {
        if f < self.lo {
            f = self.lo;
        } else if f > self.hi {
            f = self.hi;
        }
        f
    }

    fn resample_y(&mut self) {
        let Some(cy) = self.clist_y.clone() else {
            return;
        };
        let Some(clist) = cy.get(self.cur_dst_y as usize) else {
            return;
        };
        let Some(scan) = self.scan_buf.as_mut() else {
            return;
        };
        let mut dst = std::mem::take(&mut self.dst_buf);
        let mut tmp = std::mem::take(&mut self.tmp_buf);
        {
            let ptmp: &mut [Sample] = if self.delay_x_resample {
                &mut tmp
            } else {
                &mut dst
            };

            // Process each contributor.
            for (i, c) in clist.p.iter().take(clist.n as usize).enumerate() {
                // locate the contributor's location in the scan buffer -- the contributor must
                // always be found!
                let Some(j) = scan.scan_buf_y.iter().position(|&y| y == c.pixel as i32) else {
                    continue;
                };
                let src: &[Sample] = scan.scan_buf_l[j].as_deref().unwrap_or(&[]);
                if i == 0 {
                    Self::scale_y_mov(ptmp, src, c.weight, self.intermediate_x);
                } else {
                    Self::scale_y_add(ptmp, src, c.weight, self.intermediate_x);
                }

                // If this source line doesn't contribute to any more destination lines then
                // mark the scanline buffer slot which holds this source line as free.
                let p = c.pixel as usize;
                if let Some(cnt) = self.src_y_count.get_mut(p) {
                    *cnt = cnt.wrapping_sub(1);
                    if *cnt == 0 {
                        if let Some(f) = self.src_y_flag.get_mut(p) {
                            *f = 0;
                        }
                        scan.scan_buf_y[j] = -1;
                    }
                }
            }
        }

        // Now generate the destination line.
        if self.delay_x_resample {
            // X resampling was delayed until after Y resampling.
            self.resample_x(&mut dst, &tmp);
        }
        if self.lo < self.hi {
            let n = self.resample_dst_x.max(0) as usize;
            for d in dst.iter_mut().take(n) {
                *d = self.clamp_sample(*d);
            }
        }
        self.dst_buf = dst;
        self.tmp_buf = tmp;
    }

    /// Feeds the next source line; false on error or when all lines were consumed.
    pub fn put_line(&mut self, src: &[Sample]) -> bool {
        if self.cur_src_y >= self.resample_src_y {
            return false;
        }

        // Does this source line contribute to any destination line? if not, exit now.
        let cur = self.cur_src_y as usize;
        if self.src_y_count.get(cur).copied().unwrap_or(0) == 0 {
            self.cur_src_y += 1;
            return true;
        }

        let Some(mut scan) = self.scan_buf.take() else {
            return false;
        };

        // Find an empty slot in the scanline buffer.
        let Some(i) = scan.scan_buf_y.iter().position(|&y| y == -1) else {
            self.scan_buf = Some(scan);
            // If the buffer is full, exit with an error.
            self.status = Status::ScanBufferFull;
            return false;
        };

        if let Some(f) = self.src_y_flag.get_mut(cur) {
            *f = 1;
        }
        scan.scan_buf_y[i] = self.cur_src_y;

        // Does this slot have any memory allocated to it?
        if scan.scan_buf_l[i].is_none() {
            match try_vec(self.intermediate_x.max(0) as usize, 0.0) {
                Some(v) => scan.scan_buf_l[i] = Some(v),
                None => {
                    self.scan_buf = Some(scan);
                    self.status = Status::OutOfMemory;
                    return false;
                }
            }
        }

        if let Some(line) = scan.scan_buf_l[i].as_mut() {
            if self.delay_x_resample {
                // Y-X resampling order
                let n = line.len().min(src.len());
                line[..n].copy_from_slice(&src[..n]);
            } else {
                // X-Y resampling order
                self.resample_x(line, src);
            }
        }
        self.scan_buf = Some(scan);

        self.cur_src_y += 1;
        true
    }

    /// Next output line, or `None` if none is currently available (give the resampler more
    /// scanlines!).
    pub fn get_line(&mut self) -> Option<&[Sample]> {
        // If all the destination lines have been generated, then always return NULL.
        if self.cur_dst_y == self.resample_dst_y {
            return None;
        }

        // Check to see if all the required contributors are present, if not, return NULL.
        let cy = self.clist_y.as_deref()?;
        let l = cy.get(self.cur_dst_y as usize)?;
        for c in l.p.iter().take(l.n as usize) {
            if self.src_y_flag.get(c.pixel as usize).copied().unwrap_or(0) == 0 {
                return None;
            }
        }

        self.resample_y();
        self.cur_dst_y += 1;
        Some(&self.dst_buf)
    }

    /// Reinits resampler so it can handle another frame.
    pub fn restart(&mut self) {
        if self.status != Status::Okay {
            return;
        }
        self.cur_src_y = 0;
        self.cur_dst_y = 0;
        self.src_y_count.iter_mut().for_each(|v| *v = 0);
        self.src_y_flag.iter_mut().for_each(|v| *v = 0);
        self.count_src_y();
        if let Some(scan) = self.scan_buf.as_mut() {
            scan.scan_buf_y.iter_mut().for_each(|v| *v = -1);
            scan.scan_buf_l.iter_mut().for_each(|v| *v = None);
        }
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// Returned contributor lists can be shared with another Resampler.
    pub fn get_clists(&self) -> (Option<ContribLists>, Option<ContribLists>) {
        (self.clist_x.clone(), self.clist_y.clone())
    }

    pub fn get_clist_x(&self) -> Option<ContribLists> {
        self.clist_x.clone()
    }

    pub fn get_clist_y(&self) -> Option<ContribLists> {
        self.clist_y.clone()
    }

    /// True when the lists were supplied by the caller (`m_clist_*_forced`).
    pub fn clists_forced(&self) -> (bool, bool) {
        (self.clist_x_forced, self.clist_y_forced)
    }

    pub fn get_filter_num() -> i32 {
        G_RESAMPLE_FILTERS.len() as i32
    }

    pub fn get_filter_name(filter_num: i32) -> Option<&'static str> {
        if filter_num < 0 {
            return None;
        }
        G_RESAMPLE_FILTERS.get(filter_num as usize).map(|f| f.name)
    }

    /// Port of `free_clist`; lists are released when the last reference is dropped.
    pub fn free_clist(p: Option<ContribLists>) {
        drop(p);
    }

    /// Port of the destructor (`delete resampler`).
    pub fn destroy(self) {}
}

fn count_ops(clist: Option<&Vec<ContribList>>, k: i32) -> i32 {
    clist.map_or(0, |l| {
        l.iter()
            .take(k.max(0) as usize)
            .fold(0i32, |t, c| t.wrapping_add(c.n as i32))
    })
}
