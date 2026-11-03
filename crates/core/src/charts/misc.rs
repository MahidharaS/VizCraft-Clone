//! TODO

use super::RenderOpts;
use crate::doc::VisualSpec;
use crate::scene::Scene;
use crate::vquery::VisualData;

pub fn qna(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::qna")
}

pub fn script(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::script")
}

pub fn box_plot(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::box_plot")
}

pub fn bullet(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::bullet")
}

pub fn gantt(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::gantt")
}

pub fn radar(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::radar")
}

pub fn sankey(_spec: &VisualSpec, _d: &VisualData, o: &RenderOpts) -> Scene {
    super::message(o, "misc::sankey")
}

