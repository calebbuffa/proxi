//! Reusable structure-of-arrays staging buffers for the AoS convenience
//! batch methods.
//!
//! These buffers grow geometrically and never shrink in steady state, so
//! after warmup repeated AoS batch calls perform no allocation.

pub(crate) struct Scratch {
    pub(crate) xs: Vec<f64>,
    pub(crate) ys: Vec<f64>,
    pub(crate) zs: Vec<f64>,
    pub(crate) ts: Vec<f64>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

impl Scratch {
    pub(crate) fn new() -> Self {
        Self {
            xs: Vec::new(),
            ys: Vec::new(),
            zs: Vec::new(),
            ts: Vec::new(),
        }
    }

    /// Grow buffers to hold at least `n` elements each.
    /// Grows geometrically; never shrinks.
    pub(crate) fn ensure_capacity(&mut self, n: usize) {
        self.xs.reserve(n.saturating_sub(self.xs.len()));
        self.ys.reserve(n.saturating_sub(self.ys.len()));
        self.zs.reserve(n.saturating_sub(self.zs.len()));
        self.ts.reserve(n.saturating_sub(self.ts.len()));
        // `reserve` only grows; now resize to `n` so slicing works.
        self.xs.resize(n, 0.0);
        self.ys.resize(n, 0.0);
        self.zs.resize(n, 0.0);
        self.ts.resize(n, 0.0);
    }
}
