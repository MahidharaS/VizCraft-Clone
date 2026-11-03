//! TODO

use super::RenderOpts;
use crate::doc::VisualSpec;
use crate::scene::Scene;
use crate::vquery::VisualData;

pub fn key_influencers(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "ai::key_influencers")
}

pub fn decomposition(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "ai::decomposition")
}

pub fn narrative(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "ai::narrative")
}

