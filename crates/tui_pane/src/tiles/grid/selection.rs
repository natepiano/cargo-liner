use super::EndpointLayoutRules;
use super::Rect;
use super::Slot;
use super::TileContent;
use super::TileDemands;
use super::TileGrowth;
/// Measurements selected together for one destination.
#[derive(Clone)]
pub(in crate::tiles) struct SelectedLayoutMeasurements<Id> {
    pub(in crate::tiles) widths:  Vec<(TileContent<Id>, u16)>,
    pub(in crate::tiles) demands: TileDemands<Id>,
    pub(super) depth:             usize,
}

/// A depth either has a measured fit or permits no candidate.
pub(super) enum CandidateDepthSelection<Id> {
    NoFit,
    Selected(SelectedLayoutMeasurements<Id>),
}

/// `LayoutDemandMeasurement` pairs candidate content `widths` with the provider's `demands`.
#[derive(Clone, Eq)]
pub(super) struct LayoutDemandMeasurement<Id> {
    pub(super) widths:  Vec<(TileContent<Id>, u16)>,
    pub(super) demands: TileDemands<Id>,
}

impl<Id: PartialEq> PartialEq for LayoutDemandMeasurement<Id> {
    fn eq(&self, other: &Self) -> bool {
        // LayoutDemandMeasurement equality compares row requests at each measured width.
        self.widths == other.widths
            && self.demands.summary == other.demands.summary
            && self.demands.groups == other.demands.groups
    }
}

/// `LayoutSelectionInputs` records `area`, `target`, `growth`, `rules` and candidate `measurements`
/// for reuse checks.
#[derive(Eq, PartialEq)]
pub(super) struct LayoutSelectionInputs<Id> {
    pub(super) area:         Rect,
    pub(super) target:       Vec<Slot<Id>>,
    pub(super) growth:       TileGrowth,
    pub(super) rules:        EndpointLayoutRules,
    pub(super) measurements: Vec<LayoutDemandMeasurement<Id>>,
}

/// `CandidateMeasurementFreshness` names the observations available for a new search.
pub(super) enum CandidateMeasurementFreshness {
    CurrentWidth,
    AllCachedWidths,
}

impl<Id: Clone + Eq> LayoutSelectionInputs<Id> {
    /// `refresh_measurements` validates cached rows after an uncertain preferred width change.
    pub(super) fn refresh_measurements(
        &mut self,
        previous: &Self,
        measure: &mut impl FnMut(&[(TileContent<Id>, u16)]) -> TileDemands<Id>,
    ) -> CandidateMeasurementFreshness {
        let observation = self.measurements.remove(0);
        let freshness = if self.area == previous.area
            && self.target == previous.target
            && self.growth == previous.growth
            && self.rules == previous.rules
            && !previous.measurements.iter().any(|measured| {
                measured.widths == observation.widths
                    && measured.demands.summary_width == observation.demands.summary_width
            }) {
            CandidateMeasurementFreshness::AllCachedWidths
        } else {
            CandidateMeasurementFreshness::CurrentWidth
        };
        self.measurements.clone_from(&previous.measurements);
        for measured in &mut self.measurements {
            if measured.widths == observation.widths {
                measured.demands.clone_from(&observation.demands);
            } else if matches!(freshness, CandidateMeasurementFreshness::AllCachedWidths) {
                measured.demands = measure(&measured.widths);
            }
        }
        if !self
            .measurements
            .iter()
            .any(|measured| measured.widths == observation.widths)
        {
            self.measurements.push(observation);
        }
        freshness
    }
}

/// `LayoutSelection` holds the `selected` destination measurements and the `inputs` that permit
/// reuse.
pub(super) struct LayoutSelection<Id> {
    pub(super) inputs:   LayoutSelectionInputs<Id>,
    pub(super) selected: SelectedLayoutMeasurements<Id>,
}

/// A grid selects its first plan after its provider measures content.
pub(super) enum SelectedLayoutPlan<Id> {
    Unmeasured,
    Selected(Box<LayoutSelection<Id>>),
}
