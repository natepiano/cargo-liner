use ratatui::Frame;
use ratatui::layout::Rect;

use super::card;
use crate::constants::TOAST_ROW_GAP;
use crate::constants::TOAST_SIDE_GAP;
use crate::toasts::ToastFocus;
use crate::toasts::ToastHitbox;
use crate::toasts::ToastView;

#[derive(Clone, Copy)]
pub(super) struct StackLayout {
    pub(super) width: u16,
    pub(super) gap:   u16,
    pub(super) focus: ToastFocus,
}

pub(super) fn render_top_down(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    allocated: &[u16],
    layout: StackLayout,
) -> Vec<ToastHitbox> {
    let mut hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.y.saturating_add(TOAST_ROW_GAP);
    for (toast, &alloc_height) in toasts.iter().zip(allocated) {
        if alloc_height == 0 {
            continue;
        }
        let card_height = toast.desired_height().min(alloc_height);
        if card_height == 0
            || cursor_y.saturating_add(card_height) > area.bottom().saturating_sub(TOAST_ROW_GAP)
        {
            break;
        }
        let x = area
            .right()
            .saturating_sub(TOAST_SIDE_GAP)
            .saturating_sub(layout.width);
        let card = Rect {
            x,
            y: cursor_y,
            width: layout.width,
            height: card_height,
        };
        let close_rect = card::render_toast(frame, area, card, toast, layout.focus);
        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
        cursor_y = cursor_y.saturating_add(card_height + layout.gap);
    }
    hitboxes
}

pub(super) fn render_bottom_up(
    frame: &mut Frame,
    area: Rect,
    toasts: &[ToastView],
    allocated: &[u16],
    layout: StackLayout,
) -> Vec<ToastHitbox> {
    let mut hitboxes = Vec::with_capacity(toasts.len());
    let mut cursor_y = area.bottom().saturating_sub(TOAST_ROW_GAP);
    for (toast, &alloc_height) in toasts.iter().zip(allocated).rev() {
        if alloc_height == 0 {
            continue;
        }
        let card_height = toast.desired_height().min(alloc_height);
        if card_height == 0 {
            continue;
        }
        cursor_y = cursor_y.saturating_sub(card_height);
        if cursor_y < area.y.saturating_add(TOAST_ROW_GAP) {
            break;
        }
        let x = area
            .right()
            .saturating_sub(TOAST_SIDE_GAP)
            .saturating_sub(layout.width);
        let card = Rect {
            x,
            y: cursor_y,
            width: layout.width,
            height: card_height,
        };
        let close_rect = card::render_toast(frame, area, card, toast, layout.focus);
        hitboxes.push(ToastHitbox {
            id: toast.id(),
            card_rect: card,
            close_rect,
        });
        cursor_y = cursor_y.saturating_sub(layout.gap);
    }
    hitboxes.reverse();
    hitboxes
}

pub(super) fn allocate_toast_heights(toasts: &[ToastView], available: u16, gap: u16) -> Vec<u16> {
    let mut alloc = vec![0u16; toasts.len()];
    let total_min = toasts
        .iter()
        .map(ToastView::min_height)
        .fold(0u16, u16::saturating_add)
        .saturating_add(
            gap.saturating_mul(u16::try_from(toasts.len().saturating_sub(1)).unwrap_or(u16::MAX)),
        );

    if total_min > available {
        let mut used = 0u16;
        let mut drawn = 0;
        for (idx, toast) in toasts.iter().enumerate().rev() {
            let min_height = toast.min_height();
            let cost = min_height.saturating_add(if drawn > 0 { gap } else { 0 });
            if used.saturating_add(cost) <= available {
                alloc[idx] = min_height;
                used = used.saturating_add(cost);
                drawn += 1;
            }
        }
        return alloc;
    }

    for (idx, toast) in toasts.iter().enumerate() {
        alloc[idx] = toast.min_height();
    }
    let mut remaining = available.saturating_sub(total_min);
    while remaining > 0 {
        let mut changed = false;
        for (idx, toast) in toasts.iter().enumerate() {
            if remaining == 0 {
                break;
            }
            if alloc[idx] < toast.desired_height() {
                alloc[idx] += 1;
                remaining -= 1;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    alloc
}
