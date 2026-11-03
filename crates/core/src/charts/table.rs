//! TODO

use super::RenderOpts;
use crate::doc::VisualSpec;
use crate::scene::Scene;
use crate::vquery::VisualData;

pub fn table(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "table::table")
}

pub fn matrix(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "table::matrix")
}

pub fn heat(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "table::heat")
}

