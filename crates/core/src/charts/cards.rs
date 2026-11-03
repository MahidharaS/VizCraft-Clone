//! TODO

use super::RenderOpts;
use crate::doc::VisualSpec;
use crate::scene::Scene;
use crate::vquery::VisualData;

pub fn gauge(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "cards::gauge")
}

pub fn card(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "cards::card")
}

pub fn multi_row(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "cards::multi_row")
}

pub fn kpi(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "cards::kpi")
}

