//! TODO

use super::RenderOpts;
use crate::doc::VisualSpec;
use crate::scene::Scene;
use crate::vquery::VisualData;

pub fn render(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "slicer::render")
}

