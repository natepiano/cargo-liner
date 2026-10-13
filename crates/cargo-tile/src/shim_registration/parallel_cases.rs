//! Run independent process fixtures together, retaining every result and panic.

pub(super) fn run<T: Send, R: Send>(
    cases: impl IntoIterator<Item = T>,
    case: impl Fn(T) -> R + Sync,
) -> Vec<R> {
    std::thread::scope(|scope| {
        let case = &case;
        let mut children = Vec::new();
        for value in cases {
            children.push(scope.spawn(move || case(value)));
        }
        children
            .into_iter()
            .map(|child| {
                child
                    .join()
                    .unwrap_or_else(|failure| std::panic::resume_unwind(failure))
            })
            .collect()
    })
}
