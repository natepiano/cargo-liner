/// Smoothstep across `unit`: both ends where they were, and the travel
/// between them slowest at either end and fastest in the middle.
///
/// The scale comes from the caller because the two readers hold their
/// fractions on different ones -- [`DriftingText`](crate::backdrop::DriftingText) interpolates its
/// lanes in a fixed point fine enough for a lane hundreds of lines
/// deep, and [`ResolvingPixels`](crate::backdrop::ResolvingPixels) carries its coarseness on the
/// same byte the alpha it becomes is written on.
pub(in crate::backdrop) fn smoothstep(fraction: u32, unit: u32) -> u32 {
    let unit = u64::from(unit);
    let along = u64::from(fraction).min(unit);
    if unit == 0 {
        return 0;
    }
    let eased = along * along * (3 * unit - 2 * along) / (unit * unit);
    u32::try_from(eased).unwrap_or(fraction)
}
