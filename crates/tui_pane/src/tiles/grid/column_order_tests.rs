use super::ColumnRanks;
use super::TileColumnOrder;
use crate::tiles::grid::Slot;

#[test]
fn ranked_columns_keep_source_order_for_initial_ties() {
    let tied = ColumnRanks::SourceOrder.refresh(TileColumnOrder::LongestWithShortest(vec![
        (0, 5),
        (1, 5),
        (2, 5),
        (3, 5),
    ]));
    assert_eq!(
        tied.arrange(
            &[
                Slot::Group(0),
                Slot::Group(1),
                Slot::Group(2),
                Slot::Group(3)
            ],
            &[3, 2],
            1
        ),
        vec![
            Slot::Group(0),
            Slot::Group(3),
            Slot::Group(1),
            Slot::Group(2)
        ]
    );
}

#[test]
fn ranked_columns_skip_full_columns_and_preserve_empty_slots() {
    let ranks = ColumnRanks::SourceOrder.refresh(TileColumnOrder::LongestWithShortest(
        (0..5).map(|i| (i, 10 - i)).collect(),
    ));
    let slots: Vec<_> = (0..5).map(Slot::Group).collect();
    assert_eq!(
        ranks.arrange(&slots, &[3, 3], 1),
        vec![
            Slot::Group(0),
            Slot::Group(3),
            Slot::Group(1),
            Slot::Group(2),
            Slot::Group(4)
        ]
    );
    assert_eq!(
        ranks.arrange(&slots, &[3, 4], 2),
        vec![
            Slot::Group(0),
            Slot::Group(1),
            Slot::Group(2),
            Slot::Group(3),
            Slot::Group(4)
        ]
    );
    for order in [vec![0, 1, 2], vec![0, 1, 1, 2], vec![0, 99, 1, 2]] {
        let ranks = ColumnRanks::SourceOrder.refresh(TileColumnOrder::LongestWithShortest(
            order.into_iter().map(|id| (id, 10 - id.min(9))).collect(),
        ));
        let slots: Vec<_> = (0..4).map(Slot::Group).collect();
        assert_eq!(
            ranks.arrange(&slots, &[3, 2], 1),
            vec![
                Slot::Group(0),
                Slot::Group(3),
                Slot::Group(1),
                Slot::Group(2)
            ],
            "missing, repeated, and absent identities retain each slot identity once"
        );
    }

    let slots = vec![
        Slot::Group(0),
        Slot::Empty(99),
        Slot::Group(1),
        Slot::Group(2),
        Slot::Group(3),
        Slot::Group(4),
    ];
    let placed = ranks.arrange(&slots, &[4, 3], 1);
    assert_eq!(
        placed,
        vec![
            Slot::Group(0),
            Slot::Empty(99),
            Slot::Group(3),
            Slot::Group(1),
            Slot::Group(2),
            Slot::Group(4)
        ]
    );
}
